use std::collections::VecDeque;
use std::fmt::Display;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub struct Bucket {
    /// bucket fillup begin from seconds from app start
    pub time: u32,
    /// values by percentiles
    pub scale: Vec<Scale>,
    /// dof this bucket
    pub range: Range,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scale {
    /// measured aggregated value
    pub sum: u64,
    /// sums power if use
    pub power: u32,
    /// counter
    pub count: u32,
}

impl Scale {
    /// one sample
    #[inline]
    fn append(&mut self, value: u64) {
        self.add_sum(value);
        if self.count < u32::MAX {
            self.count += 1;
        }
    }

    /// safe sum
    #[inline]
    fn add_sum(&mut self, value: u64) {
        let v = u64::MAX - self.sum;
        if value >= v {
            if self.power < u32::MAX {
                self.power += 1;
            }
            self.sum = value - v;
        } else {
            self.sum += value;
        }
    }

    /// another scale's samples: its count is theirs, not one more
    #[inline]
    fn add(&mut self, value: &Self) {
        if u32::MAX - self.count > value.count {
            self.count += value.count;
        }
        if u32::MAX - self.power > value.power {
            self.power += value.power;
        }
        self.add_sum(value.sum);
    }

    #[inline]
    fn avg(&self) -> u64 {
        let power = u64::MAX as u128 * self.power as u128;
        if self.count > 0 {
            ((self.sum as u128 + power) / self.count as u128) as u64
        } else {
            0
        }
    }

}

#[derive(Clone, Debug, PartialEq)]
pub struct Range {
    /// min: 0, max: 1
    pub(crate) min_max: (u64, u64),
}

impl Display for Range {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "{}..{}", self.min_max.0, self.min_max.1)
    }
}

impl Range {
    #[inline]
    fn check(&mut self, value: u64) {
        // both: the first value is the min and the max
        if self.min_max.0 > value {
            self.min_max.0 = value;
        }
        if self.min_max.1 < value {
            self.min_max.1 = value
        }
    }

    #[inline]
    fn check_in(&self,  percentile: u8, value: u64) -> bool {
        let pp = ((self.min_max.1 - self.min_max.0)  as f32 / 200f32 * (100f32 - percentile as f32)).round() as u64;
        self.min_max.0 + pp  <= value && self.min_max.1 - pp >= value
    }
}

impl Default for Range {
    fn default() -> Self {
        Range {
            // percentiles: vec![],
            min_max: (u64::MAX - 1, 0)
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    /// aggregated percentiles configuration, 10 config max
    pub(crate) percentiles: Vec<u8>,
    /// bucket lifetime
    pub(crate) span_sec: u8,
    /// gauge lifetime
    pub(crate) live_time_sec: u16,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            percentiles: vec![],
            span_sec: 1,
            live_time_sec: 120,
        }
    }
}

impl Config {
    #[inline]
    pub fn validate(self) -> Result<(), String> {
        let mut msg = String::new();
        for p in &self.percentiles {
            Config::append(&mut msg, *p >= 100, "'percentile' mut be less than 100%");
            Config::append(&mut msg, *p <= 50, "'percentile' mut be great than 50%");
        }
        Config::append(&mut msg, self.span_sec == 0, "'span' mut be great than 0");
        Config::append(&mut msg, self.live_time_sec < self.span_sec as u16 + 1u16, "'live_time_sec' mut be great than 'span'");
        if msg.len() > 0 {
            Err(msg)
        } else {
            Ok(())
        }
    }

    #[inline]
    fn append(msg: &mut String, cnd: bool, err: &str) {
        if cnd {
            if msg.len() > 0 {
                msg.push_str(", ");
            }
            msg.push_str(err);
        }
    }

    pub fn find(&self, percentile: u8) -> Result<usize, String> {
        if percentile > 10 {
            let mut idx = 1;
            let mut found = false;
            for p in &self.percentiles {
                if *p == percentile {
                    found = true;
                    break;
                }
                idx += 1;
            }
            if found {
                Ok(idx)
            } else {
                Err(format!("cant find {}% of {}", percentile, self.percentiles.len()))
            }
        } else if self.percentiles.len() > 0
            && self.percentiles.len() > percentile as usize {
            Ok(percentile as usize + 1)
        } else {
            Err(format!("cant find #{} of {}", percentile, self.percentiles.len()))
        }
    }
}

impl Bucket {
    fn new(time: u32) -> Self {
        Bucket { time,
            scale: vec![Scale {
                sum: 0,
                power: 0,
                count: 0,
            }],
            range: Default::default(),
        }
    }
}

/// A histogram that uses plain 64bit counters for each bucket.
#[derive(Clone, Debug)]
pub struct Histogram {
    pub(crate) config: Config,
    pub(crate) start: Instant,
    pub(crate) buckets: VecDeque<Bucket>,
    /// overall range of buckets, modified on evict
    pub(crate) range: Range,
    /// overall range lifetime
    pub(crate) range_lifetime: Range,
}

impl Histogram {
    pub fn new(config: Config) -> Histogram {
        Histogram {
            config,
            start: Instant::now(),
            buckets: Default::default(),
            range: Default::default(),
            range_lifetime: Default::default(),
        }
    }

    pub fn append(&mut self, value: u64) {
         let time = self.start.elapsed().as_secs();
         let time = if time >= u32::MAX as u64 { u32::MAX } else { time  as u32};
         if self.buckets.len() == 0  || time - &self.buckets.front().unwrap().time > self.config.span_sec as u32 {
             self.buckets.push_front(Bucket::new(time))
         }
         self.range.check(value);
         self.range_lifetime.check(value);
         let b = self.buckets.front_mut().unwrap();
         b.scale.get_mut(0).unwrap().append(value);
         b.range.check(value);

         for percentile_id in 1..self.config.percentiles.len()+1 {
             if b.scale.len() <= percentile_id {
                 b.scale.push(Scale { sum: 0, power: 0, count: 0 });
             }

             if self.range.check_in(self.config.percentiles[percentile_id - 1], value) {
                 b.scale[percentile_id].append(value);
             }
         }

         // check to evict
         if self.buckets.len() > 1 && self.config.live_time_sec > 0
             && self.buckets.back().unwrap().time > self.config.live_time_sec as u32 {
             let b = &self.buckets.pop_back().unwrap();
             // the bucket held a bound of the range
             if b.range.min_max.0 <= self.range.min_max.0 || b.range.min_max.1 >= self.range.min_max.1 {

                 // modify range after evict
                 let mut r = Range::default();
                 // lookup new range
                 for x in &self.buckets {
                     if x.range.min_max.0 < r.min_max.0 {
                         r.min_max.0 = x.range.min_max.0;
                     }
                     if x.range.min_max.1 > r.min_max.1 {
                         r.min_max.1 = x.range.min_max.1;
                     }
                 }
                 self.range = r;
             }
         }

    }

    pub fn median(&self) -> u64 {
        let min = self.range.min_max.0;
        if min <= self.range.min_max.1 {
            min + (self.range.min_max.1 - min) / 2
        } else {
            0
        }
    }

    pub fn median_lt(&self) -> u64 {
        let min = self.range_lifetime.min_max.0;
        if min <= self.range_lifetime.min_max.1 {
            min + (self.range_lifetime.min_max.1 - min) / 2
        } else {
            0
        }
    }

    ///
    pub fn average(&self) -> u64 {
        let mut r = Scale { sum: 0, power: 0, count: 0 };
        for b in &self.buckets {
            r.add(&b.scale[0])
        }
        r.avg()
    }

    ///
    pub fn average_p(&self, percentile: u8) -> Result<u64, String> {
        let pid = self.config.find(percentile)?;
        let mut r = Scale { sum: 0, power: 0, count: 0 };
        for b in &self.buckets {
            r.add(&b.scale[pid])
        }
        Ok(r.avg())
    }

    pub fn buckets(&self) -> usize {
        self.buckets.len()
    }

    pub fn sample_count(&self) -> usize {
        let mut s = 0usize;
        for b in &self.buckets {
            s += b.scale[0].count as usize;
        }
        s
    }

    pub fn sample_count_p(&self, percentile: u8) -> Result<usize, String> {
        let pid = self.config.find(percentile)?;
        let mut s = 0usize;
        for b in &self.buckets {
            s += b.scale[pid].count as usize;
        }
        Ok(s)
    }

}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test() {
        // let power = u64::MAX as u128 * u32::MAX as u128;
        // assert!(power > u64::MAX as u128);
        let mut h = Histogram::new(Config::default());
        h.append(0);
        assert_eq!(h.median(), 0);
        h.append(2);
        assert_eq!(h.median(), 1);
    }

    #[test]
    fn test_p() {
        let mut h = Histogram::new(Config {
            percentiles: vec![95],
            span_sec: 1,
            live_time_sec: 100,
        });
        h.append(0);
        h.append(100);
        assert_eq!(h.median(), 50);
        for x in 1..101 {
            h.append(x);
        }
        assert_eq!(h.median(), 50);
        assert_eq!(h.average(), 50);
        assert_eq!(h.average_p(95).unwrap(), 49);
        assert_eq!(h.average_p(0 /* by index */).unwrap(), 49);
        assert_eq!(h.sample_count(), 102);
        assert_eq!(h.sample_count_p(95).unwrap(), 96);
        assert_eq!(h.median_lt(), 50);
        assert_eq!(h.buckets(), 1);
    }

    #[test]
    /// a sum past u64::MAX carries into power
    fn test_overflow() {
        let mut s = Scale { sum: u64::MAX - 1, power: 0, count: 0 };
        s.append(3);
        assert_eq!((s.sum, s.power, s.count), (2, 1, 1));
        assert_eq!(Scale { sum: 0, power: 0, count: 0 }.avg(), 0);
    }

    #[test]
    fn test_range_display() {
        assert_eq!(Range { min_max: (1, 5) }.to_string(), "1..5");
    }

    #[test]
    fn test_validate() {
        assert_eq!(Config::default().validate(), Ok(()));
        let c = Config { percentiles: vec![100, 50], span_sec: 0, live_time_sec: 0 };
        assert_eq!(c.validate().unwrap_err(), "'percentile' mut be less than 100%, \
            'percentile' mut be great than 50%, 'span' mut be great than 0, \
            'live_time_sec' mut be great than 'span'");
    }

    #[test]
    /// by percent above 10, by index up to 10
    fn test_find() {
        let c = Config { percentiles: vec![90, 95], ..Config::default() };
        assert_eq!(c.find(95), Ok(2));
        assert_eq!(c.find(99).unwrap_err(), "cant find 99% of 2");
        assert_eq!(c.find(1), Ok(2));
        assert_eq!(c.find(2).unwrap_err(), "cant find #2 of 2");
        let h = Histogram::new(Config::default());
        assert!(h.average_p(0).is_err());
        assert!(h.sample_count_p(95).is_err());
        assert_eq!(h.median(), 0, "no samples");
    }

    #[test]
    fn test_evict() {
        let mut h = Histogram::new(Config { percentiles: vec![], span_sec: 1, live_time_sec: 2 });
        h.start = h.start.checked_sub(std::time::Duration::from_secs(5)).unwrap();
        h.append(10); // a bucket at 5 s
        h.start = h.start.checked_sub(std::time::Duration::from_secs(5)).unwrap();
        h.append(20); // a bucket at 10 s: the one at 5 s is past the live time
        assert_eq!(h.buckets(), 1);
        assert_eq!(h.sample_count(), 1);
        assert_eq!(h.median(), 20, "the range without the evicted bucket");
        assert_eq!(h.median_lt(), 15, "the lifetime range keeps it");
    }

    #[test]
    /// one value is the min and the max
    fn test_one_value() {
        let mut h = Histogram::new(Config { percentiles: vec![95], ..Config::default() });
        assert_eq!((h.median(), h.median_lt()), (0, 0), "no samples");
        h.append(5);
        assert_eq!((h.median(), h.median_lt()), (5, 5));
        assert_eq!(h.average_p(95), Ok(5));
    }

}