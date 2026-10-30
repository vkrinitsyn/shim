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
    /// safe sum
    #[inline]
    fn append(&mut self, value: u64) {
        let v = u64::MAX - self.sum;
        if value >= v {
            if self.power < u32::MAX {
                self.power += 1;
            }
            self.sum = value - v;
        } else {
            self.sum += value;
        }
        if self.count < u32::MAX {
            self.count += 1;
        }
    }

    #[inline]
    fn add(&mut self, value: &Self) {
        if u32::MAX - self.count > value.count {
            self.count += value.count;
        }
        if u32::MAX - self.power > value.power {
            self.power += value.power;
        }
        self.append(value.sum);
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
        if self.min_max.0 > value {
            self.min_max.0 = value;
        } else if self.min_max.1 < value {
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
         if b.range.min_max.0 > value {
             b.range.min_max.0 = value;
         } else if b.range.min_max.1 < value {
             b.range.min_max.1 = value
         }

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
             if b.range.min_max.0 < self.range.min_max.0 || b.range.min_max.1 > self.range.min_max.1 {

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
        min + (self.range_lifetime.min_max.1 - min) / 2
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
        assert_eq!(h.average_p(95).unwrap(), 48);
        assert_eq!(h.average_p(0 /* by index */).unwrap(), 48);
        assert_eq!(h.sample_count(), 102);
        assert_eq!(h.sample_count_p(95).unwrap(), 96);
    }

    #[test]
    fn test_empty() {
        let h = Histogram::new(Config::default());
        assert_eq!(h.buckets(), 0);
        assert_eq!(h.sample_count(), 0);
        assert_eq!(h.average(), 0);
        // no value yet: min is above max
        assert_eq!(h.median(), 0);
    }

    #[test]
    fn test_median_lt() {
        let mut h = Histogram::new(Config::default());
        h.append(10);
        h.append(30);
        assert_eq!(h.median_lt(), 20);
        assert_eq!(h.buckets(), 1);
    }

    #[test]
    fn test_scale_overflow() {
        let mut s = Scale { sum: u64::MAX - 1, power: 0, count: 0 };
        s.append(5);
        assert_eq!(s, Scale { sum: 4, power: 1, count: 1 });
        assert_eq!(s.avg(), ((u64::MAX as u128 + 4) / 1) as u64);

        // power and count stay at their max
        let mut s = Scale { sum: u64::MAX, power: u32::MAX, count: u32::MAX };
        s.append(1);
        assert_eq!(s, Scale { sum: 1, power: u32::MAX, count: u32::MAX });

        // add: the counts summed while they fit, the sums carried into power
        let mut s = Scale { sum: u64::MAX - 1, power: 1, count: 2 };
        s.add(&Scale { sum: 3, power: 2, count: 3 });
        assert_eq!(s, Scale { sum: 2, power: 4, count: 6 });
        let mut s = Scale { sum: 0, power: u32::MAX, count: u32::MAX };
        s.add(&Scale { sum: 1, power: 1, count: 1 });
        assert_eq!(s, Scale { sum: 1, power: u32::MAX, count: u32::MAX });
    }

    #[test]
    fn test_range() {
        let mut r = Range::default();
        r.check(10);
        r.check(20);
        r.check(5);
        assert_eq!(r.to_string(), "5..20");
        // 90%: 5% cut from each end of 5..20
        assert!(r.check_in(90, 6));
        assert!(r.check_in(90, 19));
        assert!(!r.check_in(90, 5));
        assert!(!r.check_in(60, 7));
    }

    #[test]
    fn test_validate() {
        assert_eq!(Config::default().validate(), Ok(()));
        assert_eq!(Config { percentiles: vec![95], span_sec: 1, live_time_sec: 2 }.validate(), Ok(()));
        assert_eq!(Config { percentiles: vec![100, 50], span_sec: 0, live_time_sec: 0 }.validate(),
            Err("'percentile' mut be less than 100%, 'percentile' mut be great than 50%, \
                 'span' mut be great than 0, 'live_time_sec' mut be great than 'span'".to_string()));
        assert_eq!(Config { percentiles: vec![], span_sec: 5, live_time_sec: 5 }.validate(),
            Err("'live_time_sec' mut be great than 'span'".to_string()));
    }

    #[test]
    fn test_find() {
        let c = Config { percentiles: vec![90, 95, 99], span_sec: 1, live_time_sec: 120 };
        assert_eq!(c.find(90), Ok(1));
        assert_eq!(c.find(99), Ok(3));
        assert_eq!(c.find(2 /* by index */), Ok(3));
        assert_eq!(c.find(80), Err("cant find 80% of 3".to_string()));
        assert_eq!(c.find(3), Err("cant find #3 of 3".to_string()));
        assert_eq!(Config::default().find(0), Err("cant find #0 of 0".to_string()));

        let h = Histogram::new(c);
        assert!(h.average_p(80).is_err());
        assert!(h.sample_count_p(80).is_err());
    }

    #[test]
    fn test_evict() {
        let mut h = Histogram::new(Config { percentiles: vec![], span_sec: 1, live_time_sec: 2 });
        // nine seconds in: the newest bucket is this second's, the oldest one past its lifetime
        h.start = Instant::now() - std::time::Duration::from_secs(9);
        let mut front = Bucket::new(9);
        front.range = Range { min_max: (10, 20) };
        let mut back = Bucket::new(5);
        back.range = Range { min_max: (0, 2000) };
        h.buckets.push_front(back);
        h.buckets.push_front(front);
        h.range = Range { min_max: (5, 1000) };

        h.append(15);
        assert_eq!(h.buckets(), 1);
        // the range is looked up again from the buckets left
        assert_eq!(h.range, Range { min_max: (10, 20) });
        assert_eq!(h.median(), 15);
        assert_eq!(h.sample_count(), 1);
    }

    #[test]
    fn test_new_bucket() {
        let mut h = Histogram::new(Config { percentiles: vec![], span_sec: 1, live_time_sec: 0 });
        h.start = Instant::now() - std::time::Duration::from_secs(5);
        h.buckets.push_front(Bucket::new(1));
        h.append(7);
        // a bucket older than the span: a new one in front, nothing evicted with no lifetime
        assert_eq!(h.buckets(), 2);
        assert_eq!(h.buckets.front().unwrap().time, 5);
        assert_eq!(h.sample_count(), 1);
    }

}