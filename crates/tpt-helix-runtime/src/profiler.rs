//! Runtime profiler (TODO.md Phase 2 → Developer tooling).
//!
//! A small, dependency-free instrumentation registry that the runtime's
//! subsystems can report into. It covers the three measurements named in the
//! TODO item:
//!
//! * **Memory** — caller-reported byte gauges with a high-water mark
//!   (`record_memory`). The profiler does not probe the OS heap itself; the
//!   owning subsystem reports the size of what it holds (e.g. the decoded-frame
//!   buffer or the layout tree), which keeps the numbers deterministic and
//!   headless-testable.
//! * **Layout ops/sec** — counted operations plus accumulated wall time per
//!   named channel (`count` / `span`), from which `ops_per_sec` is derived.
//! * **WASM execution overhead** — timed spans around guest calls
//!   (`span("wasm.call")`), giving total and mean host-observed cost.
//!
//! Recording methods take `&self` and use interior mutability, so spans can be
//! nested (an outer `wasm.call` span may contain layout spans) and a profiler
//! can be shared by reference between subsystems. It is single-threaded by
//! design, matching the `RuntimeStub` model.
//!
//! Timing is opt-in per `Profiler` instance; a disabled profiler is a cheap
//! no-op so hot paths can keep their instrumentation in release builds.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Accumulated statistics for one named channel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChannelStats {
    /// Number of operations recorded (spans count as one op each).
    pub ops: u64,
    /// Total wall time spent inside timed spans on this channel.
    pub total: Duration,
}

impl ChannelStats {
    /// Mean wall time per op, or `None` when nothing was timed.
    ///
    /// Computed in nanoseconds with 128-bit intermediates, so large op counts
    /// do not distort the mean.
    pub fn mean(&self) -> Option<Duration> {
        if self.ops == 0 || self.total.is_zero() {
            return None;
        }
        let nanos = self.total.as_nanos() / u128::from(self.ops);
        Some(Duration::from_nanos(
            u64::try_from(nanos).unwrap_or(u64::MAX),
        ))
    }

    /// Operations per second over the timed wall time, or `None` when no time
    /// was recorded.
    pub fn ops_per_sec(&self) -> Option<f64> {
        let secs = self.total.as_secs_f64();
        if secs > 0.0 {
            Some(self.ops as f64 / secs)
        } else {
            None
        }
    }
}

/// Memory gauge with a high-water mark.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryGauge {
    /// Most recently reported size in bytes.
    pub current: u64,
    /// Largest size ever reported.
    pub peak: u64,
}

/// Instrumentation registry. Each runtime owns one and shares it by reference.
#[derive(Debug, Default)]
pub struct Profiler {
    enabled: bool,
    channels: RefCell<BTreeMap<String, ChannelStats>>,
    memory: RefCell<BTreeMap<String, MemoryGauge>>,
}

/// RAII guard returned by [`Profiler::span`]; records elapsed time on drop.
///
/// Holds a shared reference, so spans nest freely and several can be alive at
/// once.
pub struct Span<'a> {
    profiler: Option<&'a Profiler>,
    channel: &'static str,
    start: Option<Instant>,
}

impl Profiler {
    /// Creates an enabled profiler.
    pub fn new() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    /// Creates a disabled profiler: every recording call becomes a no-op.
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Whether recording is active.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enables or disables recording. Existing data is kept.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Counts `n` operations on `channel` without timing them.
    pub fn count(&self, channel: &str, n: u64) {
        if !self.enabled {
            return;
        }
        self.with_channel(channel, |s| s.ops += n);
    }

    /// Adds an externally measured duration (one op) to `channel`.
    pub fn record_duration(&self, channel: &str, elapsed: Duration) {
        if !self.enabled {
            return;
        }
        self.with_channel(channel, |s| {
            s.ops += 1;
            s.total += elapsed;
        });
    }

    /// Times the closure as one op on `channel` and returns its result.
    pub fn time<T>(&self, channel: &str, f: impl FnOnce() -> T) -> T {
        if !self.enabled {
            return f();
        }
        let start = Instant::now();
        let out = f();
        self.record_duration(channel, start.elapsed());
        out
    }

    /// Starts a timed span on a static channel name; the span is recorded when
    /// the returned guard is dropped. Spans may be nested.
    pub fn span(&self, channel: &'static str) -> Span<'_> {
        if self.enabled {
            Span {
                profiler: Some(self),
                channel,
                start: Some(Instant::now()),
            }
        } else {
            Span {
                profiler: None,
                channel,
                start: None,
            }
        }
    }

    /// Reports the current size of a memory consumer and updates its peak.
    pub fn record_memory(&self, consumer: &str, bytes: u64) {
        if !self.enabled {
            return;
        }
        let mut memory = self.memory.borrow_mut();
        let gauge = memory.entry(consumer.to_owned()).or_default();
        gauge.current = bytes;
        gauge.peak = gauge.peak.max(bytes);
    }

    /// Statistics for a channel, if it has been recorded.
    pub fn channel(&self, name: &str) -> Option<ChannelStats> {
        self.channels.borrow().get(name).cloned()
    }

    /// Gauge for a memory consumer, if it has been reported.
    pub fn memory(&self, consumer: &str) -> Option<MemoryGauge> {
        self.memory.borrow().get(consumer).copied()
    }

    /// All channels in name order.
    pub fn channels(&self) -> Vec<(String, ChannelStats)> {
        self.channels
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Clears all recorded data (the enabled flag is unchanged).
    pub fn reset(&self) {
        self.channels.borrow_mut().clear();
        self.memory.borrow_mut().clear();
    }

    /// Human-readable summary for logs and CI output.
    pub fn report_text(&self) -> String {
        let mut out = String::from("== runtime profile ==\n");
        for (name, s) in self.channels.borrow().iter() {
            out.push_str(&format!("{name}: ops={}", s.ops));
            if let Some(ops) = s.ops_per_sec() {
                out.push_str(&format!(" ops/sec={ops:.1}"));
            }
            if let Some(mean) = s.mean() {
                out.push_str(&format!(" mean={mean:?}"));
            }
            out.push('\n');
        }
        for (name, g) in self.memory.borrow().iter() {
            out.push_str(&format!(
                "memory[{name}]: current={}B peak={}B\n",
                g.current, g.peak
            ));
        }
        out
    }

    fn with_channel(&self, channel: &str, f: impl FnOnce(&mut ChannelStats)) {
        let mut channels = self.channels.borrow_mut();
        let stats = match channels.get_mut(channel) {
            Some(s) => s,
            None => channels.entry(channel.to_owned()).or_default(),
        };
        f(stats);
    }
}

impl Drop for Span<'_> {
    fn drop(&mut self) {
        if let (Some(profiler), Some(start)) = (self.profiler, self.start) {
            profiler.record_duration(self.channel, start.elapsed());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_profiler_records_nothing() {
        let p = Profiler::disabled();
        p.count("layout", 5);
        p.record_memory("tree", 100);
        {
            let _s = p.span("wasm.call");
        }
        assert!(p.channel("layout").is_none());
        assert!(p.channel("wasm.call").is_none());
        assert!(p.memory("tree").is_none());
    }

    #[test]
    fn counts_accumulate_without_timing() {
        let p = Profiler::new();
        p.count("layout", 3);
        p.count("layout", 2);
        let s = p.channel("layout").unwrap();
        assert_eq!(s.ops, 5);
        assert_eq!(s.total, Duration::ZERO);
        assert_eq!(s.ops_per_sec(), None);
        assert_eq!(s.mean(), None);
    }

    #[test]
    fn recorded_durations_drive_mean_and_ops_per_sec() {
        let p = Profiler::new();
        p.record_duration("layout", Duration::from_millis(10));
        p.record_duration("layout", Duration::from_millis(30));
        let s = p.channel("layout").unwrap();
        assert_eq!(s.ops, 2);
        assert_eq!(s.total, Duration::from_millis(40));
        assert_eq!(s.mean(), Some(Duration::from_millis(20)));
        // 2 ops over 0.04 s = 50 ops/sec.
        let ops = s.ops_per_sec().unwrap();
        assert!((ops - 50.0).abs() < 1e-9, "got {ops}");
    }

    #[test]
    fn mean_is_not_clamped_for_more_than_u32_max_ops() {
        // Synthetic stats: 2^33 ops over 2^33 ns total → mean should be 1 ns.
        // The old u32 clamp would divide by u32::MAX and report ~4 ns.
        let s = ChannelStats {
            ops: 1u64 << 33,
            total: Duration::from_nanos(1u64 << 33),
        };
        assert_eq!(s.mean(), Some(Duration::from_nanos(1)));
    }

    #[test]
    fn span_records_one_op_on_drop() {
        let p = Profiler::new();
        {
            let _s = p.span("wasm.call");
            std::thread::sleep(Duration::from_millis(2));
        }
        let s = p.channel("wasm.call").unwrap();
        assert_eq!(s.ops, 1);
        assert!(s.total >= Duration::from_millis(2));
    }

    #[test]
    fn spans_nest_and_each_records_its_own_op() {
        let p = Profiler::new();
        {
            let _outer = p.span("wasm.call");
            {
                let _inner = p.span("layout");
                std::thread::sleep(Duration::from_millis(1));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let outer = p.channel("wasm.call").unwrap();
        let inner = p.channel("layout").unwrap();
        assert_eq!(outer.ops, 1);
        assert_eq!(inner.ops, 1);
        // The outer span covers the inner one.
        assert!(outer.total >= inner.total);
    }

    #[test]
    fn time_returns_closure_value_and_records() {
        let p = Profiler::new();
        let v = p.time("layout", || 7 * 6);
        assert_eq!(v, 42);
        assert_eq!(p.channel("layout").unwrap().ops, 1);
    }

    #[test]
    fn memory_gauge_tracks_peak_separately_from_current() {
        let p = Profiler::new();
        p.record_memory("frames", 100);
        p.record_memory("frames", 300);
        p.record_memory("frames", 50);
        assert_eq!(
            p.memory("frames"),
            Some(MemoryGauge {
                current: 50,
                peak: 300
            })
        );
    }

    #[test]
    fn reset_clears_data_but_keeps_enabled_flag() {
        let p = Profiler::new();
        p.count("layout", 1);
        p.record_memory("tree", 9);
        p.reset();
        assert!(p.channel("layout").is_none());
        assert!(p.memory("tree").is_none());
        assert!(p.is_enabled());
    }

    #[test]
    fn report_text_lists_channels_and_memory() {
        let p = Profiler::new();
        p.record_duration("wasm.call", Duration::from_millis(5));
        p.record_memory("tree", 2048);
        let text = p.report_text();
        assert!(text.contains("wasm.call: ops=1"));
        assert!(text.contains("memory[tree]: current=2048B peak=2048B"));
    }
}
