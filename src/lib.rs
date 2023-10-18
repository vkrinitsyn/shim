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
        self.count += value.count;
        self.power += value.power;
        self.append(value.sum);
    }

    #[inline]
    fn avg(&self) -> u64 {
        let power = u64::MAX as u128 * self.power as u128;
        ((self.sum as u128 + power) / self.count as u128) as u64
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
         if self.buckets.len() > 1 && self.buckets.back().unwrap().time > self.config.live_time_sec as u32 {
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

    pub fn average(&self) -> u64 {
        let min = self.range.min_max.0;
        min + (self.range.min_max.1 - min) / 2
    }

    pub fn average_lt(&self) -> u64 {
        let min = self.range_lifetime.min_max.0;
        min + (self.range_lifetime.min_max.1 - min) / 2
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
        let mut h = Histogram::new(Config::default());
        h.append(0);
        assert_eq!(h.average(), 0);
        h.append(2);
        assert_eq!(h.average(), 1);
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
        assert_eq!(h.average(), 50);
        for x in 1..100 {
            h.append(x);
        }
        assert_eq!(h.average(), 50);
        assert_eq!(h.average_p(95).unwrap(), 48);
        assert_eq!(h.average_p(0 /* by index */).unwrap(), 48);
        assert_eq!(h.sample_count(), 101);
        assert_eq!(h.sample_count_p(95).unwrap(), 96);

    }

}