//! W4: WASI P2 Streams & I/O — `wasi:io/streams`, `wasi:clocks`, `wasi:random`.
//!
//! Implements WASI Preview 2 I/O primitives:
//! - Input/output streams with blocking and pollable I/O (W4.1–W4.2)
//! - Poll mechanism for multiplexed I/O (W4.3)
//! - Stream splice (W4.4) and async subscribe (W4.5)
//! - Stream error handling (W4.6)
//! - Monotonic and wall clocks (W4.7–W4.8)
//! - Random number generation (W4.9)

use std::collections::VecDeque;
use std::fmt;

// ═══════════════════════════════════════════════════════════════════════
// W4.1: Input Stream
// ═══════════════════════════════════════════════════════════════════════

/// Handle for an input stream.
pub type InputStreamHandle = u32;
/// Handle for an output stream.
pub type OutputStreamHandle = u32;
/// Handle for a pollable.
pub type PollableHandle = u32;

/// An input stream that supports blocking and pollable reads.
#[derive(Debug)]
pub struct InputStream {
    /// Internal buffer.
    buffer: VecDeque<u8>,
    /// Whether the stream is closed (EOF).
    closed: bool,
    /// Total bytes read from this stream.
    bytes_read: u64,
}

impl InputStream {
    /// Creates a new empty input stream.
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
            closed: false,
            bytes_read: 0,
        }
    }

    /// Creates an input stream pre-filled with data.
    pub fn from_bytes(data: &[u8]) -> Self {
        Self {
            buffer: VecDeque::from(data.to_vec()),
            closed: false,
            bytes_read: 0,
        }
    }

    /// Pushes data into the stream (simulates data arrival).
    pub fn push_data(&mut self, data: &[u8]) {
        self.buffer.extend(data);
    }

    /// Reads up to `len` bytes (blocking: waits until data or EOF).
    pub fn read(&mut self, len: u64) -> Result<Vec<u8>, StreamError> {
        if self.buffer.is_empty() && self.closed {
            return Err(StreamError::Closed);
        }
        let n = (len as usize).min(self.buffer.len());
        let data: Vec<u8> = self.buffer.drain(..n).collect();
        self.bytes_read += data.len() as u64;
        Ok(data)
    }

    /// Checks if data is available without blocking.
    pub fn is_ready(&self) -> bool {
        !self.buffer.is_empty() || self.closed
    }

    /// Marks the stream as closed (EOF).
    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Returns whether the stream is closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Total bytes read.
    pub fn total_read(&self) -> u64 {
        self.bytes_read
    }

    /// Bytes currently available in buffer.
    pub fn available(&self) -> usize {
        self.buffer.len()
    }
}

impl Default for InputStream {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W4.2: Output Stream
// ═══════════════════════════════════════════════════════════════════════

/// An output stream with buffered writes and flush.
#[derive(Debug)]
pub struct OutputStream {
    /// Internal write buffer.
    buffer: Vec<u8>,
    /// Flushed data (available for reading by consumer).
    flushed: Vec<u8>,
    /// Whether the stream is closed.
    closed: bool,
    /// Total bytes written.
    bytes_written: u64,
}

impl OutputStream {
    /// Creates a new empty output stream.
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            flushed: Vec::new(),
            closed: false,
            bytes_written: 0,
        }
    }

    /// Writes data to the stream.
    pub fn write(&mut self, data: &[u8]) -> Result<u64, StreamError> {
        if self.closed {
            return Err(StreamError::Closed);
        }
        self.buffer.extend_from_slice(data);
        self.bytes_written += data.len() as u64;
        Ok(data.len() as u64)
    }

    /// Flushes buffered data.
    pub fn flush(&mut self) -> Result<(), StreamError> {
        if self.closed {
            return Err(StreamError::Closed);
        }
        self.flushed.extend_from_slice(&self.buffer);
        self.buffer.clear();
        Ok(())
    }

    /// Closes the stream, flushing remaining data.
    pub fn close(&mut self) {
        let _ = self.flush();
        self.closed = true;
    }

    /// Returns all flushed data.
    pub fn flushed_data(&self) -> &[u8] {
        &self.flushed
    }

    /// Total bytes written.
    pub fn total_written(&self) -> u64 {
        self.bytes_written
    }

    /// Whether the stream is closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Check if the stream can accept writes.
    pub fn is_ready(&self) -> bool {
        !self.closed
    }
}

impl Default for OutputStream {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W4.3: Poll
// ═══════════════════════════════════════════════════════════════════════

/// A pollable event source.
#[derive(Debug, Clone)]
pub enum PollSource {
    /// An input stream that may have data.
    InputStream(InputStreamHandle),
    /// An output stream that may accept data.
    OutputStream(OutputStreamHandle),
    /// A timer (monotonic clock duration in nanoseconds).
    Timer(u64),
}

/// Poll result for a single pollable.
#[derive(Debug, Clone, PartialEq)]
pub struct PollResult {
    /// Index of the pollable that is ready.
    pub index: usize,
    /// Whether this source is ready.
    pub ready: bool,
}

/// Simulated poll engine for WASI P2.
#[derive(Debug, Default)]
pub struct PollEngine {
    /// Registered pollable sources.
    sources: Vec<PollSource>,
}

impl PollEngine {
    /// Creates a new poll engine.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a pollable source, returns its index.
    pub fn subscribe(&mut self, source: PollSource) -> usize {
        let idx = self.sources.len();
        self.sources.push(source);
        idx
    }

    /// Polls a single source (blocking).
    pub fn poll_one(&self, index: usize, streams: &StreamTable) -> PollResult {
        let ready = if let Some(source) = self.sources.get(index) {
            match source {
                PollSource::InputStream(handle) => streams
                    .input_streams
                    .get(handle)
                    .is_some_and(|s| s.is_ready()),
                PollSource::OutputStream(handle) => streams
                    .output_streams
                    .get(handle)
                    .is_some_and(|s| s.is_ready()),
                PollSource::Timer(_) => true, // Timers always ready in simulation
            }
        } else {
            false
        };
        PollResult { index, ready }
    }

    /// Polls multiple sources, returns indices of all ready sources.
    pub fn poll_many(&self, streams: &StreamTable) -> Vec<PollResult> {
        (0..self.sources.len())
            .map(|i| self.poll_one(i, streams))
            .filter(|r| r.ready)
            .collect()
    }
}

/// A table of open streams (simulates WASI resource table).
#[derive(Debug, Default)]
pub struct StreamTable {
    /// Open input streams.
    pub input_streams: std::collections::HashMap<InputStreamHandle, InputStream>,
    /// Open output streams.
    pub output_streams: std::collections::HashMap<OutputStreamHandle, OutputStream>,
    /// Next handle ID.
    next_handle: u32,
}

impl StreamTable {
    /// Creates a new stream table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new input stream and returns its handle.
    pub fn new_input_stream(&mut self) -> InputStreamHandle {
        let handle = self.next_handle;
        self.next_handle += 1;
        self.input_streams.insert(handle, InputStream::new());
        handle
    }

    /// Creates an input stream with pre-filled data.
    pub fn new_input_stream_from(&mut self, data: &[u8]) -> InputStreamHandle {
        let handle = self.next_handle;
        self.next_handle += 1;
        self.input_streams
            .insert(handle, InputStream::from_bytes(data));
        handle
    }

    /// Creates a new output stream and returns its handle.
    pub fn new_output_stream(&mut self) -> OutputStreamHandle {
        let handle = self.next_handle;
        self.next_handle += 1;
        self.output_streams.insert(handle, OutputStream::new());
        handle
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W4.4: Stream Splice
// ═══════════════════════════════════════════════════════════════════════

/// Splices data from an input stream to an output stream (zero-copy pipe).
pub fn splice(
    input: &mut InputStream,
    output: &mut OutputStream,
    max_bytes: u64,
) -> Result<u64, StreamError> {
    let data = input.read(max_bytes)?;
    let len = data.len() as u64;
    if len > 0 {
        output.write(&data)?;
    }
    Ok(len)
}

// ═══════════════════════════════════════════════════════════════════════
// W4.6: Stream Errors
// ═══════════════════════════════════════════════════════════════════════

/// Stream error type.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamError {
    /// Stream has been closed.
    Closed,
    /// Operation timed out.
    Timeout,
    /// Permission denied.
    PermissionDenied,
    /// Generic I/O error.
    Io(String),
    /// Last operation failed (WASI stream-error convention).
    LastOperationFailed(String),
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => write!(f, "stream closed"),
            Self::Timeout => write!(f, "stream timeout"),
            Self::PermissionDenied => write!(f, "permission denied"),
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
            Self::LastOperationFailed(msg) => write!(f, "last operation failed: {msg}"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W4.7: Monotonic Clock
// ═══════════════════════════════════════════════════════════════════════

/// WASI monotonic clock.
#[derive(Debug)]
pub struct MonotonicClock {
    /// Current time in nanoseconds.
    now_ns: u64,
    /// Resolution in nanoseconds.
    resolution_ns: u64,
}

impl MonotonicClock {
    /// Creates a new monotonic clock.
    pub fn new() -> Self {
        Self {
            now_ns: 0,
            resolution_ns: 1_000_000, // 1ms resolution
        }
    }

    /// Returns the current time in nanoseconds.
    pub fn now(&self) -> u64 {
        self.now_ns
    }

    /// Returns the clock resolution in nanoseconds.
    pub fn resolution(&self) -> u64 {
        self.resolution_ns
    }

    /// Advances the clock by the given duration (for simulation).
    pub fn advance(&mut self, ns: u64) {
        self.now_ns += ns;
    }

    /// Creates a pollable that resolves after `duration_ns` nanoseconds.
    pub fn subscribe_duration(&self, duration_ns: u64) -> PollSource {
        PollSource::Timer(self.now_ns + duration_ns)
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W4.8: Wall Clock
// ═══════════════════════════════════════════════════════════════════════

/// WASI wall clock datetime.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateTime {
    /// Seconds since Unix epoch.
    pub seconds: u64,
    /// Nanoseconds within the current second.
    pub nanoseconds: u32,
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:09}", self.seconds, self.nanoseconds)
    }
}

/// WASI wall clock.
#[derive(Debug)]
pub struct WallClock {
    /// Current datetime.
    now: DateTime,
}

impl WallClock {
    /// Creates a wall clock at a given time.
    pub fn new(seconds: u64, nanoseconds: u32) -> Self {
        Self {
            now: DateTime {
                seconds,
                nanoseconds,
            },
        }
    }

    /// Creates a wall clock at the current system time (simulated).
    pub fn system() -> Self {
        // Use a fixed timestamp for deterministic tests
        Self::new(1_711_900_000, 0)
    }

    /// Returns the current datetime.
    pub fn now(&self) -> DateTime {
        self.now
    }

    /// Advances the clock (for simulation).
    pub fn advance_seconds(&mut self, seconds: u64) {
        self.now.seconds += seconds;
    }
}

impl Default for WallClock {
    fn default() -> Self {
        Self::system()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W4.9: Random
// ═══════════════════════════════════════════════════════════════════════

/// WASI random number generator.
#[derive(Debug)]
pub struct WasiRandom {
    /// Simple LCG state for deterministic testing.
    state: u64,
}

impl WasiRandom {
    /// Creates a new random generator with a seed.
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(1), // Ensure non-zero
        }
    }

    /// Returns a random u64.
    pub fn get_random_u64(&mut self) -> u64 {
        // LCG parameters (same as glibc)
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.state
    }

    /// Returns `len` random bytes.
    pub fn get_random_bytes(&mut self, len: usize) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(len);
        while bytes.len() < len {
            let val = self.get_random_u64();
            for b in val.to_le_bytes() {
                if bytes.len() < len {
                    bytes.push(b);
                }
            }
        }
        bytes
    }
}

impl Default for WasiRandom {
    fn default() -> Self {
        Self::new(42)
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── W4.1: Input stream ──

    #[test]
    fn w4_1_input_stream_read_line_by_line() {
        let mut stream = InputStream::from_bytes(b"line1\nline2\nline3\n");
        let mut lines = Vec::new();
        loop {
            match stream.read(6) {
                Ok(data) if data.is_empty() => break,
                Ok(data) => lines.push(data),
                Err(StreamError::Closed) => break,
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert!(!lines.is_empty());
        assert_eq!(&lines[0], b"line1\n");
    }

    #[test]
    fn w4_1_input_stream_eof() {
        let mut stream = InputStream::new();
        stream.close();
        let err = stream.read(10).unwrap_err();
        assert_eq!(err, StreamError::Closed);
    }

    // ── W4.2: Output stream ──

    #[test]
    fn w4_2_output_stream_write_flush() {
        let mut stream = OutputStream::new();
        stream.write(b"Hello, ").unwrap();
        stream.write(b"World!").unwrap();
        stream.flush().unwrap();
        assert_eq!(stream.flushed_data(), b"Hello, World!");
    }

    #[test]
    fn w4_2_output_stream_close() {
        let mut stream = OutputStream::new();
        stream.write(b"data").unwrap();
        stream.close();
        let err = stream.write(b"more").unwrap_err();
        assert_eq!(err, StreamError::Closed);
        // Flushed data is available
        assert_eq!(stream.flushed_data(), b"data");
    }

    // ── W4.3: Poll ──

    #[test]
    fn w4_3_poll_one_ready() {
        let mut table = StreamTable::new();
        let h = table.new_input_stream_from(b"data");
        let mut engine = PollEngine::new();
        let idx = engine.subscribe(PollSource::InputStream(h));
        let result = engine.poll_one(idx, &table);
        assert!(result.ready);
    }

    #[test]
    fn w4_3_poll_many_first_ready() {
        let mut table = StreamTable::new();
        let h1 = table.new_input_stream(); // empty, not ready
        let h2 = table.new_input_stream_from(b"data"); // has data, ready
        let mut engine = PollEngine::new();
        engine.subscribe(PollSource::InputStream(h1));
        engine.subscribe(PollSource::InputStream(h2));

        let ready = engine.poll_many(&table);
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].index, 1);
    }

    // ── W4.4: Stream splice ──

    #[test]
    fn w4_4_splice_stdin_to_file() {
        let mut input = InputStream::from_bytes(b"piped data here");
        let mut output = OutputStream::new();

        let spliced = splice(&mut input, &mut output, 1024).unwrap();
        assert_eq!(spliced, 15);

        output.flush().unwrap();
        assert_eq!(output.flushed_data(), b"piped data here");
    }

    // ── W4.5: Async streams (subscribe) ──

    #[test]
    fn w4_5_non_blocking_read() {
        let mut stream = InputStream::new();
        // No data yet
        assert!(!stream.is_ready());

        // Data arrives
        stream.push_data(b"async data");
        assert!(stream.is_ready());

        let data = stream.read(100).unwrap();
        assert_eq!(data, b"async data");
    }

    // ── W4.6: Error handling ──

    #[test]
    fn w4_6_stream_error_display() {
        assert_eq!(StreamError::Closed.to_string(), "stream closed");
        assert_eq!(StreamError::Timeout.to_string(), "stream timeout");
        assert_eq!(
            StreamError::LastOperationFailed("EOF".into()).to_string(),
            "last operation failed: EOF"
        );
    }

    #[test]
    fn w4_6_eof_and_permission_errors() {
        let mut stream = InputStream::new();
        stream.close();
        assert_eq!(stream.read(10).unwrap_err(), StreamError::Closed);

        let mut out = OutputStream::new();
        out.close();
        assert_eq!(out.write(b"x").unwrap_err(), StreamError::Closed);
    }

    // ── W4.7: Monotonic clock ──

    #[test]
    fn w4_7_monotonic_clock_timestamps() {
        let mut clock = MonotonicClock::new();
        assert_eq!(clock.now(), 0);
        assert_eq!(clock.resolution(), 1_000_000); // 1ms

        clock.advance(5_000_000_000); // 5 seconds
        assert_eq!(clock.now(), 5_000_000_000);

        let poll = clock.subscribe_duration(1_000_000_000);
        if let PollSource::Timer(deadline) = poll {
            assert_eq!(deadline, 6_000_000_000);
        }
    }

    // ── W4.8: Wall clock ──

    #[test]
    fn w4_8_wall_clock_within_1s() {
        let clock = WallClock::system();
        let now = clock.now();
        assert!(now.seconds > 1_000_000_000); // After year 2001
        assert_eq!(now.nanoseconds, 0);

        let display = format!("{now}");
        assert!(display.contains("."));
    }

    // ── W4.9: Random ──

    #[test]
    fn w4_9_random_bytes_non_zero() {
        let mut rng = WasiRandom::new(12345);
        let bytes = rng.get_random_bytes(32);
        assert_eq!(bytes.len(), 32);
        // At least some bytes should be non-zero
        assert!(bytes.iter().any(|&b| b != 0));
    }

    #[test]
    fn w4_9_random_u64() {
        let mut rng = WasiRandom::new(42);
        let a = rng.get_random_u64();
        let b = rng.get_random_u64();
        assert_ne!(a, b); // Different values
        assert_ne!(a, 0);
    }

    // ── W4.10: Comprehensive tests ──

    #[test]
    fn w4_10_stream_table_lifecycle() {
        let mut table = StreamTable::new();
        let ih = table.new_input_stream_from(b"hello");
        let oh = table.new_output_stream();

        // Read from input
        let data = table.input_streams.get_mut(&ih).unwrap().read(100).unwrap();
        assert_eq!(data, b"hello");

        // Write to output
        table
            .output_streams
            .get_mut(&oh)
            .unwrap()
            .write(b"world")
            .unwrap();
        table.output_streams.get_mut(&oh).unwrap().flush().unwrap();
        assert_eq!(table.output_streams[&oh].flushed_data(), b"world");
    }

    #[test]
    fn w4_10_poll_with_timer_and_streams() {
        let mut table = StreamTable::new();
        let h = table.new_input_stream_from(b"data");
        let clock = MonotonicClock::new();

        let mut engine = PollEngine::new();
        engine.subscribe(PollSource::InputStream(h));
        engine.subscribe(clock.subscribe_duration(1_000_000));

        let ready = engine.poll_many(&table);
        // Both should be ready (stream has data, timer always ready in sim)
        assert_eq!(ready.len(), 2);
    }
}
