# shim
Sliding Window of average timeframe with percentiles calculation 



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
        assert_eq!(h.average_p(95).unwrap(), 48);
        assert_eq!(h.average_p(0 /* by index */).unwrap(), 48);
        assert_eq!(h.sample_count(), 102);
        assert_eq!(h.sample_count_p(95).unwrap(), 96);
    }
