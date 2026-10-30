# shim

[![ci](https://github.com/vkrinitsyn/shim/actions/workflows/ci.yml/badge.svg)](https://github.com/vkrinitsyn/shim/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/vkrinitsyn/shim/graph/badge.svg)](https://codecov.io/gh/vkrinitsyn/shim)
[![audit](https://github.com/vkrinitsyn/shim/actions/workflows/audit.yml/badge.svg)](https://github.com/vkrinitsyn/shim/actions/workflows/audit.yml)
[![crates.io](https://img.shields.io/crates/v/shims.svg)](https://crates.io/crates/shims)
[![docs.rs](https://docs.rs/shims/badge.svg)](https://docs.rs/shims)

Sliding Window of average timeframe with percentiles calculation 

> [!NOTE]
> Provided by [DBinvent](https://www.dbinvent.com), the infinite-scale Postgres solution.



    #[test]
    /// simple test
    fn test() {
        let mut h = Histogram::new(Config::default());
        h.append(0);
        assert_eq!(h.median(), 0);
        h.append(2);
        assert_eq!(h.median(), 1);
    }

    #[test]
    /// complex test
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
    }
