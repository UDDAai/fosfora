//! Decoder worker for streamed video.
//!
//! One worker thread per clip sits between a byte stream of tightly packed RGBA8 frames
//! (an ffmpeg pipe in the app, an in-memory reader in tests) and the consumer. It reads
//! exactly `width * height * 4` bytes per frame into a buffer from a fixed pool, tags it
//! `(epoch, index)` as [`super::stream_clock::StreamClock`] expects, and sends it over a
//! queue of [`QUEUE_DEPTH`] frames. A full queue blocks the worker, so a paused clip
//! costs no CPU, and RAM is [`StreamDecoder::resident_bytes`] whatever the clip length.
//!
//! The stream comes from a producer closure `(start_secs, looping) -> FrameSource`. A
//! [`FrameSource`] is a reader plus a `kill` hook. The producer is called once at spawn
//! and again after every seek with the new start, always on the reader thread, so a
//! slow spawn never delays a seek or Drop. A looping stream is the producer's job: it
//! should never reach EOF, and EOF while looping is an error. EOF before the first
//! frame of an epoch is an error too. [`StreamDecoder::set_looping`] only changes the
//! flag handed to the next producer call.
//!
//! Frames come back to the pool when the [`PooledBuffer`] inside them is dropped, so no
//! path (seek, shutdown, a frame the consumer discards) can leak a buffer.
//!
//! Commands (seek, looping, shutdown) interrupt a send blocked on a full queue. Seeks
//! already waiting when the worker looks collapse into the latest one; a seek that
//! arrives while an earlier one is being served gets its own reader. A reader that
//! has not yet started its producer call when it is abandoned never makes it. A seek
//! also drains the queue, so no frame of an older epoch arrives after the first frame
//! of the new one. After [`StreamMsg::End`] or [`StreamMsg::Error`] the worker idles
//! until a seek restarts it.
//!
//! Blocking reads: the reader and the producer live on their own thread, so the worker
//! never waits on either. A seek or shutdown abandons that thread and, at once and
//! synchronously, calls the source's `kill` hook (for ffmpeg: kill the child), which
//! makes a blocked pipe read return so the thread exits through the abort path. The
//! hook runs on the worker thread and must be quick and must not block. If the
//! abandoned thread is still inside the producer, the hook runs as soon as the
//! producer returns. A producer panic becomes an error message. The source must block
//! on reads (`WouldBlock` is retried after a short sleep).
//!
//! Not yet wired into the app.
#![allow(dead_code)]

use std::io::{self, Read};
use std::ops::Deref;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Select, Sender, TryRecvError, bounded, select, unbounded};

/// Frames buffered between the worker and the consumer.
pub const QUEUE_DEPTH: usize = 3;
/// Frame buffers allocated once per decoder: the queue, one being read, one in the
/// worker's hands and a couple for the consumer.
pub const POOL_BUFFERS: usize = 6;

/// A byte stream of tightly packed RGBA8 frames and the way to stop it.
pub struct FrameSource {
    /// Blocking reader of the frames.
    pub reader: Box<dyn Read + Send>,
    /// Makes a blocked `reader.read` return (kill the ffmpeg child). Called once when
    /// the stream is abandoned (seek or Drop); keep it quick and non-blocking.
    pub kill: Box<dyn FnOnce() + Send>,
}

/// Opens the stream for `(start_secs, looping)`.
pub type Producer = Box<dyn FnMut(f64, bool) -> Result<FrameSource, String> + Send>;

/// Fixed set of frame buffers shared by the worker, the reader and the consumer.
struct Pool {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    allocations: AtomicUsize,
}

impl Pool {
    fn new(frame_bytes: usize) -> Arc<Self> {
        let (tx, rx) = bounded(POOL_BUFFERS);
        let pool = Arc::new(Self {
            tx,
            rx,
            allocations: AtomicUsize::new(0),
        });
        for _ in 0..POOL_BUFFERS {
            pool.allocations.fetch_add(1, Ordering::Relaxed);
            let _ = pool.tx.send(vec![0u8; frame_bytes]);
        }
        pool
    }
}

/// A frame buffer on loan from the pool; dropping it returns it.
pub struct PooledBuffer {
    data: Vec<u8>,
    pool: Arc<Pool>,
}

impl PooledBuffer {
    fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }
}

impl Deref for PooledBuffer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.data
    }
}

impl std::fmt::Debug for PooledBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PooledBuffer({} bytes)", self.data.len())
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        let data = std::mem::take(&mut self.data);
        let _ = self.pool.tx.try_send(data);
    }
}

/// One decoded frame: RGBA8, `width * height * 4` bytes.
#[derive(Debug)]
pub struct StreamFrame {
    pub epoch: u64,
    /// Frames since the start of the epoch, from 0.
    pub index: u64,
    pub data: PooledBuffer,
}

/// What the worker tells the consumer.
#[derive(Debug)]
pub enum StreamMsg {
    Frame(StreamFrame),
    /// The stream ended cleanly (non-looping clip).
    End {
        epoch: u64,
    },
    /// The producer failed, or the stream broke (a short read mid-frame included).
    Error {
        epoch: u64,
        message: String,
    },
}

enum Command {
    Seek { epoch: u64, start_secs: f64 },
    SetLooping(bool),
    Shutdown,
}

/// What the reader thread tells the worker.
enum ReadMsg {
    Frame(PooledBuffer),
    Eof,
    Failed(String),
}

/// Handle to the worker thread of one streamed clip.
pub struct StreamDecoder {
    cmd_tx: Sender<Command>,
    out_rx: Receiver<StreamMsg>,
    pool: Arc<Pool>,
    frame_bytes: usize,
    thread: Option<JoinHandle<()>>,
}

impl StreamDecoder {
    /// Start streaming at `start_secs`, tagging frames with `epoch` (take both from the
    /// clock). Fails on a zero or overflowing frame size or when the thread cannot be
    /// spawned. A producer failure is reported as [`StreamMsg::Error`] instead.
    pub fn spawn<P>(
        width: u32,
        height: u32,
        epoch: u64,
        start_secs: f64,
        looping: bool,
        producer: P,
    ) -> Result<Self, String>
    where
        P: FnMut(f64, bool) -> Result<FrameSource, String> + Send + 'static,
    {
        let frame_bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(4))
            .filter(|&b| b > 0 && b.checked_mul(POOL_BUFFERS).is_some())
            .ok_or_else(|| format!("Invalid stream frame size {width}x{height}"))?;
        let pool = Pool::new(frame_bytes);
        let (cmd_tx, cmd_rx) = unbounded();
        let (out_tx, out_rx) = bounded(QUEUE_DEPTH);

        let worker = Worker {
            cmd_rx,
            out_tx,
            // The worker keeps a receiver to drain stale frames on a seek.
            out_rx: out_rx.clone(),
            pool: pool.clone(),
            frame_bytes,
            producer: Arc::new(Mutex::new(Box::new(producer))),
            epoch,
            start_secs,
            looping,
            stream_looping: looping,
            index: 0,
            session: None,
            pending_seek: None,
            shutdown: false,
        };
        let thread = std::thread::Builder::new()
            .name("stream-decoder".into())
            .spawn(move || worker.run())
            .map_err(|e| format!("Failed to spawn stream decoder thread: {e}"))?;

        Ok(Self {
            cmd_tx,
            out_rx,
            pool,
            frame_bytes,
            thread: Some(thread),
        })
    }

    /// Restart the stream at `start_secs` under `epoch` (the value from
    /// `StreamClock::seek`). Frames of older epochs stop arriving.
    pub fn seek(&self, epoch: u64, start_secs: f64) {
        let _ = self.cmd_tx.send(Command::Seek { epoch, start_secs });
    }

    /// Flag handed to the producer on its next call; a running stream is not restarted.
    pub fn set_looping(&self, looping: bool) {
        let _ = self.cmd_tx.send(Command::SetLooping(looping));
    }

    /// Non-blocking read of the next message.
    pub fn try_recv(&self) -> Option<StreamMsg> {
        self.out_rx.try_recv().ok()
    }

    /// Wait up to `timeout` for the next message.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<StreamMsg> {
        self.out_rx.recv_timeout(timeout).ok()
    }

    /// Messages waiting in the queue (never more than [`QUEUE_DEPTH`]).
    pub fn queued(&self) -> usize {
        self.out_rx.len()
    }

    /// Bytes of one frame.
    pub fn frame_bytes(&self) -> usize {
        self.frame_bytes
    }

    /// RAM held by the frame pool: fixed, whatever the clip length.
    pub fn resident_bytes(&self) -> usize {
        POOL_BUFFERS * self.frame_bytes
    }

    /// Buffers allocated so far. Stays at [`POOL_BUFFERS`] for the life of the decoder.
    pub fn pool_allocations(&self) -> usize {
        self.pool.allocations.load(Ordering::Relaxed)
    }
}

impl Drop for StreamDecoder {
    /// Never waits on a read: the worker only blocks in `select!`/`Select` calls that
    /// listen for the shutdown command, and reads run on a separate thread.
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(Command::Shutdown);
        if let Some(handle) = self.thread.take()
            && handle.join().is_err()
        {
            log::error!("Stream decoder worker panicked");
        }
    }
}

/// Where the reader thread's kill hook is: it exists only once the producer returned.
enum KillSlot {
    Pending,
    Armed(Box<dyn FnOnce() + Send>),
    Abandoned,
}

/// The reader thread of the current epoch. Dropping it abandons the thread and kills
/// its source.
struct Session {
    rx: Receiver<ReadMsg>,
    /// Never sent on; dropping it tells the reader to stop.
    _abort: Sender<()>,
    kill: Arc<Mutex<KillSlot>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let prev = std::mem::replace(&mut *lock(&self.kill), KillSlot::Abandoned);
        if let KillSlot::Armed(kill) = prev {
            kill();
        }
    }
}

/// Lock that survives poisoning (the data is plain state, never left half-updated).
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Worker {
    cmd_rx: Receiver<Command>,
    out_tx: Sender<StreamMsg>,
    out_rx: Receiver<StreamMsg>,
    pool: Arc<Pool>,
    frame_bytes: usize,
    producer: Arc<Mutex<Producer>>,
    epoch: u64,
    start_secs: f64,
    looping: bool,
    /// The `looping` flag the current stream was opened with.
    stream_looping: bool,
    /// Index of the next frame of the current epoch.
    index: u64,
    session: Option<Session>,
    pending_seek: Option<(u64, f64)>,
    shutdown: bool,
}

impl Worker {
    fn run(mut self) {
        self.open();
        loop {
            // Take every waiting command; the latest seek wins.
            loop {
                match self.cmd_rx.try_recv() {
                    Ok(cmd) => self.apply(cmd),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.shutdown = true;
                        break;
                    }
                }
            }
            if self.shutdown {
                return;
            }
            if let Some((epoch, start_secs)) = self.pending_seek.take() {
                self.restart(epoch, start_secs);
                continue;
            }

            let Some(rx) = self.session.as_ref().map(|s| s.rx.clone()) else {
                // Idle after End/Error (or a failed open): wait for a command.
                match self.cmd_rx.recv() {
                    Ok(cmd) => self.apply(cmd),
                    Err(_) => self.shutdown = true,
                }
                continue;
            };
            let cmd_rx = self.cmd_rx.clone();
            select! {
                recv(cmd_rx) -> cmd => match cmd {
                    Ok(cmd) => self.apply(cmd),
                    Err(_) => self.shutdown = true,
                },
                recv(rx) -> msg => self.on_read(msg.ok()),
            }
        }
    }

    fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Seek { epoch, start_secs } => self.pending_seek = Some((epoch, start_secs)),
            Command::SetLooping(looping) => self.looping = looping,
            Command::Shutdown => self.shutdown = true,
        }
    }

    /// Drop the reader, discard queued frames of the old epoch and open the new one.
    fn restart(&mut self, epoch: u64, start_secs: f64) {
        self.session = None;
        while self.out_rx.try_recv().is_ok() {}
        self.epoch = epoch;
        self.start_secs = start_secs;
        self.open();
    }

    /// Start a reader thread; it calls the producer, then reads the stream.
    fn open(&mut self) {
        self.index = 0;
        self.stream_looping = self.looping;
        let (tx, rx) = bounded(1);
        let (abort_tx, abort_rx) = bounded(0);
        let kill = Arc::new(Mutex::new(KillSlot::Pending));
        let reader = Reader {
            producer: self.producer.clone(),
            start_secs: self.start_secs,
            looping: self.looping,
            frame_bytes: self.frame_bytes,
            pool: self.pool.clone(),
            kill: kill.clone(),
            tx,
            abort_rx,
        };
        let spawned = std::thread::Builder::new()
            .name("stream-reader".into())
            .spawn(move || reader.run());
        match spawned {
            Ok(_) => {
                self.session = Some(Session {
                    rx,
                    _abort: abort_tx,
                    kill,
                });
            }
            Err(e) => self.fail(format!("Failed to spawn stream reader thread: {e}")),
        }
    }

    fn on_read(&mut self, msg: Option<ReadMsg>) {
        let epoch = self.epoch;
        match msg {
            Some(ReadMsg::Frame(data)) => {
                let index = self.index;
                self.index += 1;
                self.deliver(StreamMsg::Frame(StreamFrame { epoch, index, data }));
            }
            Some(ReadMsg::Eof) if self.index == 0 => {
                self.fail("stream ended before the first frame".to_string());
            }
            Some(ReadMsg::Eof) if self.stream_looping => {
                self.fail("looping stream ended".to_string());
            }
            Some(ReadMsg::Eof) => {
                self.session = None;
                self.deliver(StreamMsg::End { epoch });
            }
            Some(ReadMsg::Failed(message)) => self.fail(message),
            None => self.fail("stream reader stopped unexpectedly".to_string()),
        }
    }

    fn fail(&mut self, message: String) {
        log::warn!("Stream decoder: {message}");
        self.session = None;
        self.deliver(StreamMsg::Error {
            epoch: self.epoch,
            message,
        });
    }

    /// Send to the consumer. A full queue blocks here, but a seek or shutdown
    /// interrupts the wait and the message is dropped (its buffer returns to the pool).
    fn deliver(&mut self, msg: StreamMsg) {
        let out_tx = self.out_tx.clone();
        let cmd_rx = self.cmd_rx.clone();
        loop {
            let mut sel = Select::new();
            let i_out = sel.send(&out_tx);
            let _i_cmd = sel.recv(&cmd_rx);
            let op = sel.select();
            if op.index() == i_out {
                let _ = op.send(&out_tx, msg);
                return;
            }
            match op.recv(&cmd_rx) {
                Ok(cmd) => self.apply(cmd),
                Err(_) => self.shutdown = true,
            }
            if self.shutdown || self.pending_seek.is_some() {
                return;
            }
        }
    }
}

/// Everything the reader thread of one epoch needs.
struct Reader {
    producer: Arc<Mutex<Producer>>,
    start_secs: f64,
    looping: bool,
    frame_bytes: usize,
    pool: Arc<Pool>,
    kill: Arc<Mutex<KillSlot>>,
    tx: Sender<ReadMsg>,
    abort_rx: Receiver<()>,
}

impl Reader {
    /// Call the producer, arm the kill hook, then fill pool buffers until EOF, an error
    /// or abandonment. Owns the reader, so it is dropped when this returns.
    fn run(self) {
        let source = {
            let mut producer = lock(&self.producer);
            // Abandoned while waiting for an earlier producer call: skip ours.
            if abandoned(&self.abort_rx) {
                return;
            }
            let (start_secs, looping) = (self.start_secs, self.looping);
            match catch_unwind(AssertUnwindSafe(|| (*producer)(start_secs, looping))) {
                Ok(Ok(source)) => source,
                Ok(Err(message)) => {
                    let _ = self.tx.send(ReadMsg::Failed(message));
                    return;
                }
                Err(_) => {
                    let _ = self
                        .tx
                        .send(ReadMsg::Failed("producer panicked".to_string()));
                    return;
                }
            }
        };
        let FrameSource { reader, kill } = source;
        {
            let mut slot = lock(&self.kill);
            if matches!(*slot, KillSlot::Abandoned) {
                drop(slot);
                kill();
                return;
            }
            *slot = KillSlot::Armed(kill);
        }
        self.read_loop(reader);
    }

    fn read_loop(&self, mut reader: Box<dyn Read + Send>) {
        debug_assert!(self.frame_bytes > 0);
        loop {
            // The abort channel never carries a message: it is ready when it disconnects.
            let data = select! {
                recv(self.pool.rx) -> data => match data {
                    Ok(data) => data,
                    Err(_) => return,
                },
                recv(self.abort_rx) -> _ => return,
            };
            let mut buf = PooledBuffer {
                data,
                pool: self.pool.clone(),
            };
            let msg = match fill_frame(&mut reader, buf.as_mut_slice(), &self.abort_rx) {
                Ok(Fill::Frame) => ReadMsg::Frame(buf),
                Ok(Fill::Eof) => ReadMsg::Eof,
                Ok(Fill::Abandoned) => return,
                Err(message) => ReadMsg::Failed(message),
            };
            let last = !matches!(msg, ReadMsg::Frame(_));
            if self.tx.send(msg).is_err() || last {
                return;
            }
        }
    }
}

fn abandoned(abort_rx: &Receiver<()>) -> bool {
    matches!(abort_rx.try_recv(), Err(TryRecvError::Disconnected))
}

enum Fill {
    Frame,
    /// Clean EOF on a frame boundary.
    Eof,
    Abandoned,
}

/// Read exactly `buf.len()` bytes. EOF before the first byte is a clean end; EOF later
/// is a partial frame.
fn fill_frame(
    source: &mut Box<dyn Read + Send>,
    buf: &mut [u8],
    abort_rx: &Receiver<()>,
) -> Result<Fill, String> {
    let mut filled = 0;
    while filled < buf.len() {
        if abandoned(abort_rx) {
            return Ok(Fill::Abandoned);
        }
        match source.read(&mut buf[filled..]) {
            Ok(0) if filled == 0 => return Ok(Fill::Eof),
            Ok(0) => {
                return Err(format!(
                    "partial frame: stream ended after {filled} of {} bytes",
                    buf.len()
                ));
            }
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => return Err(format!("stream read error: {e}")),
        }
    }
    Ok(Fill::Frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::AtomicU64;
    use std::time::Instant;

    const W: u32 = 2;
    const H: u32 = 2;
    const FRAME_BYTES: usize = 16;
    const WAIT: Duration = Duration::from_secs(5);

    /// In-memory stream: `total` frames, then `partial` stray bytes, then EOF. Frame `i`
    /// starts with `start_secs as u8` and `i` as u32 little endian. Reads come in chunks
    /// of at most 5 bytes so frames are assembled from several reads.
    struct FakeStream {
        start_tag: u8,
        total: u64,
        partial: usize,
        next: u64,
        pos: usize,
        started: Arc<AtomicU64>,
    }

    impl Read for FakeStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let in_frame = if self.next < self.total {
                FRAME_BYTES
            } else {
                self.partial
            };
            if self.pos >= in_frame {
                return Ok(0);
            }
            if self.pos == 0 && self.next < self.total {
                self.started.fetch_add(1, Ordering::SeqCst);
            }
            let n = buf.len().min(5).min(in_frame - self.pos);
            for (k, byte) in buf[..n].iter_mut().enumerate() {
                let at = self.pos + k;
                *byte = match at {
                    0 => self.start_tag,
                    1..=4 => (self.next as u32).to_le_bytes()[at - 1],
                    _ => 0xAB,
                };
            }
            self.pos += n;
            if self.pos == FRAME_BYTES && self.next < self.total {
                self.next += 1;
                self.pos = 0;
            }
            Ok(n)
        }
    }

    /// A reader that blocks until its kill hook fires (like an ffmpeg pipe and child).
    struct BlockingReader(Receiver<()>);

    impl Read for BlockingReader {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            let _ = self.0.recv();
            Ok(0)
        }
    }

    /// A blocking source; `fired` counts its kill hook running.
    fn blocking(fired: &Arc<AtomicUsize>) -> FrameSource {
        let (tx, rx) = bounded::<()>(0);
        let fired = fired.clone();
        FrameSource {
            reader: Box::new(BlockingReader(rx)),
            kill: Box::new(move || {
                fired.fetch_add(1, Ordering::SeqCst);
                drop(tx);
            }),
        }
    }

    /// Fails with `WouldBlock` twice before each read that delivers data.
    struct WouldBlockStream(FakeStream, u32);

    impl Read for WouldBlockStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.1 += 1;
            if !self.1.is_multiple_of(3) {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            self.0.read(buf)
        }
    }

    type Calls = Arc<Mutex<Vec<(f64, bool)>>>;

    fn fake_stream(start: f64, total: u64, partial: usize, started: &Arc<AtomicU64>) -> FakeStream {
        FakeStream {
            start_tag: start as u8,
            total,
            partial,
            next: 0,
            pos: 0,
            started: started.clone(),
        }
    }

    fn fake(start: f64, total: u64, partial: usize, started: &Arc<AtomicU64>) -> FrameSource {
        FrameSource {
            reader: Box::new(fake_stream(start, total, partial, started)),
            kill: Box::new(|| {}),
        }
    }

    /// Producer of `total` frames per call that records its arguments.
    fn producer(
        total: u64,
        calls: &Calls,
    ) -> impl FnMut(f64, bool) -> Result<FrameSource, String> + Send + use<> {
        let calls = calls.clone();
        let started = Arc::new(AtomicU64::new(0));
        move |start, looping| {
            calls.lock().unwrap().push((start, looping));
            Ok(fake(start, total, 0, &started))
        }
    }

    fn calls() -> Calls {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn spawn(total: u64, calls: &Calls) -> StreamDecoder {
        StreamDecoder::spawn(W, H, 0, 0.0, false, producer(total, calls)).unwrap()
    }

    fn next(dec: &StreamDecoder) -> StreamMsg {
        dec.recv_timeout(WAIT)
            .expect("timed out waiting for a message")
    }

    fn next_frame(dec: &StreamDecoder) -> StreamFrame {
        match next(dec) {
            StreamMsg::Frame(f) => f,
            other => panic!("expected a frame, got {other:?}"),
        }
    }

    fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
        let deadline = Instant::now() + WAIT;
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn index_in_data(f: &StreamFrame) -> u32 {
        u32::from_le_bytes(f.data[1..5].try_into().unwrap())
    }

    #[test]
    fn frames_arrive_in_order_then_end() {
        let dec = spawn(20, &calls());
        for i in 0..20u64 {
            let f = next_frame(&dec);
            assert_eq!((f.epoch, f.index), (0, i));
            assert_eq!(f.data.len(), FRAME_BYTES);
            assert_eq!(index_in_data(&f), i as u32);
        }
        assert!(matches!(next(&dec), StreamMsg::End { epoch: 0 }));
        // Idle after the end: nothing more arrives.
        assert!(dec.recv_timeout(Duration::from_millis(100)).is_none());
    }

    #[test]
    fn queue_is_bounded_and_buffers_are_reused() {
        let started = Arc::new(AtomicU64::new(0));
        let s = started.clone();
        let dec = StreamDecoder::spawn(W, H, 0, 0.0, false, move |start, _| {
            Ok(fake(start, 1_000_000, 0, &s))
        })
        .unwrap();

        // Stalled consumer: the queue fills to exactly QUEUE_DEPTH and reading stops.
        wait_for("a full queue", || dec.queued() == QUEUE_DEPTH);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(dec.queued(), QUEUE_DEPTH);
        assert!(started.load(Ordering::SeqCst) <= POOL_BUFFERS as u64);

        // Draining: buffers cycle through the pool, never more than POOL_BUFFERS.
        let mut seen = HashSet::new();
        let mut max_queued = 0;
        for i in 0..500u64 {
            max_queued = max_queued.max(dec.queued());
            let f = next_frame(&dec);
            assert_eq!(f.index, i);
            seen.insert(f.data.as_ptr() as usize);
        }
        assert!(max_queued <= QUEUE_DEPTH);
        assert!(
            seen.len() <= POOL_BUFFERS,
            "{} distinct buffers",
            seen.len()
        );
        assert_eq!(dec.pool_allocations(), POOL_BUFFERS);
    }

    #[test]
    fn seek_switches_epoch_and_drops_old_frames() {
        let calls = calls();
        let dec = spawn(1_000_000, &calls);
        assert_eq!(next_frame(&dec).index, 0);
        assert_eq!(next_frame(&dec).index, 1);

        dec.seek(7, 12.0);
        loop {
            let f = next_frame(&dec);
            if f.epoch == 7 {
                assert_eq!(f.index, 0);
                assert_eq!(f.data[0], 12);
                break;
            }
            assert_eq!(f.epoch, 0);
        }
        for i in 1..30u64 {
            let f = next_frame(&dec);
            assert_eq!((f.epoch, f.index), (7, i));
            assert_eq!(f.data[0], 12);
        }
        assert_eq!(*calls.lock().unwrap(), vec![(0.0, false), (12.0, false)]);
    }

    #[test]
    fn queued_seeks_coalesce_into_the_latest() {
        let calls = calls();
        let (gate_tx, gate_rx) = bounded::<()>(0);
        let started = Arc::new(AtomicU64::new(0));
        let rec = calls.clone();
        let mut first = true;
        let dec = StreamDecoder::spawn(W, H, 0, 0.0, false, move |start, looping| {
            rec.lock().unwrap().push((start, looping));
            if std::mem::take(&mut first) {
                // Hold the worker inside its first producer call while seeks pile up.
                let _ = gate_rx.recv();
            }
            Ok(fake(start, 1_000_000, 0, &started))
        })
        .unwrap();

        wait_for("the first producer call", || {
            calls.lock().unwrap().len() == 1
        });
        dec.seek(1, 1.0);
        dec.seek(2, 2.0);
        dec.seek(3, 3.0);
        // Let the worker abandon the readers of seeks 1 and 2 before the gate opens.
        std::thread::sleep(Duration::from_millis(200));
        gate_tx.send(()).unwrap();

        let f = next_frame(&dec);
        assert_eq!((f.epoch, f.index), (3, 0));
        for _ in 0..10 {
            assert_eq!(next_frame(&dec).epoch, 3);
        }
        assert_eq!(*calls.lock().unwrap(), vec![(0.0, false), (3.0, false)]);
    }

    #[test]
    fn set_looping_reaches_the_next_producer_call() {
        let calls = calls();
        let dec = spawn(5, &calls);
        wait_for("the first producer call", || {
            calls.lock().unwrap().len() == 1
        });
        dec.set_looping(true);
        dec.seek(1, 4.0);
        loop {
            if next_frame_or_end(&dec) == 1 {
                break;
            }
        }
        assert_eq!(*calls.lock().unwrap(), vec![(0.0, false), (4.0, true)]);
    }

    /// Epoch of the next frame or End/Error.
    fn next_frame_or_end(dec: &StreamDecoder) -> u64 {
        match next(dec) {
            StreamMsg::Frame(f) => f.epoch,
            StreamMsg::End { epoch } | StreamMsg::Error { epoch, .. } => epoch,
        }
    }

    #[test]
    fn seek_after_end_restarts() {
        let calls = calls();
        let dec = spawn(3, &calls);
        for _ in 0..3 {
            next_frame(&dec);
        }
        assert!(matches!(next(&dec), StreamMsg::End { epoch: 0 }));
        dec.seek(1, 2.0);
        for i in 0..3u64 {
            let f = next_frame(&dec);
            assert_eq!((f.epoch, f.index, f.data[0]), (1, i, 2));
        }
        assert!(matches!(next(&dec), StreamMsg::End { epoch: 1 }));
    }

    #[test]
    fn short_read_mid_frame_is_an_error() {
        let started = Arc::new(AtomicU64::new(0));
        let dec = StreamDecoder::spawn(W, H, 0, 0.0, false, move |start, _| {
            Ok(fake(start, 2, 7, &started))
        })
        .unwrap();
        assert_eq!(next_frame(&dec).index, 0);
        assert_eq!(next_frame(&dec).index, 1);
        match next(&dec) {
            StreamMsg::Error { epoch: 0, message } => assert!(message.contains("partial")),
            other => panic!("expected a partial-frame error, got {other:?}"),
        }
        // No corrupt frame and no End follow.
        assert!(dec.recv_timeout(Duration::from_millis(100)).is_none());
    }

    #[test]
    fn producer_error_is_reported_and_a_seek_recovers() {
        let started = Arc::new(AtomicU64::new(0));
        let mut n = 0;
        let dec = StreamDecoder::spawn(W, H, 0, 0.0, false, move |start, _| {
            n += 1;
            if n == 1 {
                Err("boom".to_string())
            } else {
                Ok(fake(start, 3, 0, &started))
            }
        })
        .unwrap();
        match next(&dec) {
            StreamMsg::Error { epoch: 0, message } => assert_eq!(message, "boom"),
            other => panic!("expected an error, got {other:?}"),
        }
        assert!(dec.recv_timeout(Duration::from_millis(100)).is_none());

        dec.seek(1, 0.0);
        for i in 0..3u64 {
            let f = next_frame(&dec);
            assert_eq!((f.epoch, f.index), (1, i));
        }
        assert!(matches!(next(&dec), StreamMsg::End { epoch: 1 }));
    }

    #[test]
    fn drop_with_a_full_queue_is_prompt() {
        let dec = spawn(1_000_000, &calls());
        wait_for("a full queue", || dec.queued() == QUEUE_DEPTH);
        std::thread::sleep(Duration::from_millis(50)); // worker is now blocked on send
        let t = Instant::now();
        drop(dec);
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    }

    #[test]
    fn drop_while_the_reader_blocks_is_prompt() {
        let fired = Arc::new(AtomicUsize::new(0));
        let calls = calls();
        let (rec, f) = (calls.clone(), fired.clone());
        let dec = StreamDecoder::spawn(W, H, 0, 0.0, false, move |start, looping| {
            rec.lock().unwrap().push((start, looping));
            Ok(blocking(&f))
        })
        .unwrap();
        wait_for("the producer call", || calls.lock().unwrap().len() == 1);
        std::thread::sleep(Duration::from_millis(50)); // reader is now blocked in read

        let t = Instant::now();
        drop(dec);
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
        // The kill hook ran synchronously inside Drop.
        assert_eq!(fired.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn drop_while_the_producer_is_slow_is_prompt() {
        let started = Arc::new(AtomicU64::new(0));
        let dec = StreamDecoder::spawn(W, H, 0, 0.0, false, move |start, _| {
            std::thread::sleep(Duration::from_secs(2));
            Ok(fake(start, 4, 0, &started))
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(50)); // producer is now sleeping
        let t = Instant::now();
        drop(dec);
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    }

    #[test]
    fn many_seeks_over_blocked_readers_fire_every_kill_hook() {
        const SEEKS: u64 = 20;
        let fired = Arc::new(AtomicUsize::new(0));
        let calls = calls();
        let started = Arc::new(AtomicU64::new(0));
        let (rec, f) = (calls.clone(), fired.clone());
        let dec = StreamDecoder::spawn(W, H, 0, 0.0, false, move |start, looping| {
            let n = {
                let mut c = rec.lock().unwrap();
                c.push((start, looping));
                c.len() as u64
            };
            // Calls 1..=SEEKS block; the last one streams.
            if n <= SEEKS {
                Ok(blocking(&f))
            } else {
                Ok(fake(start, 1_000_000, 0, &started))
            }
        })
        .unwrap();
        for k in 1..=SEEKS {
            wait_for("a producer call", || {
                calls.lock().unwrap().len() as u64 == k
            });
            dec.seek(k, k as f64);
        }
        wait_for("the last kill hook", || {
            fired.load(Ordering::SeqCst) == SEEKS as usize
        });
        // Frames flow from the final epoch, and buffers keep cycling past the pool size.
        for i in 0..(4 * POOL_BUFFERS as u64) {
            let f = next_frame(&dec);
            assert_eq!((f.epoch, f.index), (SEEKS, i));
        }
        assert_eq!(dec.pool_allocations(), POOL_BUFFERS);
        assert_eq!(calls.lock().unwrap().len() as u64, SEEKS + 1);
        assert_eq!(fired.load(Ordering::SeqCst), SEEKS as usize);
    }

    #[test]
    fn producer_panic_is_an_error_and_a_seek_recovers() {
        let started = Arc::new(AtomicU64::new(0));
        let mut n = 0;
        let dec = StreamDecoder::spawn(W, H, 0, 0.0, false, move |start, _| {
            n += 1;
            assert!(n != 1, "test panic in producer");
            Ok(fake(start, 3, 0, &started))
        })
        .unwrap();
        match next(&dec) {
            StreamMsg::Error { epoch: 0, message } => assert_eq!(message, "producer panicked"),
            other => panic!("expected an error, got {other:?}"),
        }
        dec.seek(1, 0.0);
        for i in 0..3u64 {
            let f = next_frame(&dec);
            assert_eq!((f.epoch, f.index), (1, i));
        }
        assert!(matches!(next(&dec), StreamMsg::End { epoch: 1 }));
    }

    #[test]
    fn eof_before_the_first_frame_is_an_error() {
        let dec = spawn(0, &calls());
        match next(&dec) {
            StreamMsg::Error { epoch: 0, message } => {
                assert_eq!(message, "stream ended before the first frame");
            }
            other => panic!("expected an error, got {other:?}"),
        }
        assert!(dec.recv_timeout(Duration::from_millis(100)).is_none());
    }

    #[test]
    fn eof_while_looping_is_an_error() {
        let started = Arc::new(AtomicU64::new(0));
        let dec = StreamDecoder::spawn(W, H, 0, 0.0, true, move |start, _| {
            Ok(fake(start, 3, 0, &started))
        })
        .unwrap();
        for i in 0..3u64 {
            assert_eq!(next_frame(&dec).index, i);
        }
        match next(&dec) {
            StreamMsg::Error { epoch: 0, message } => assert!(message.contains("looping")),
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[test]
    fn would_block_reads_are_retried() {
        let started = Arc::new(AtomicU64::new(0));
        let dec = StreamDecoder::spawn(W, H, 0, 0.0, false, move |start, _| {
            Ok(FrameSource {
                reader: Box::new(WouldBlockStream(fake_stream(start, 3, 0, &started), 0)),
                kill: Box::new(|| {}),
            })
        })
        .unwrap();
        for i in 0..3u64 {
            assert_eq!(next_frame(&dec).index, i);
        }
        assert!(matches!(next(&dec), StreamMsg::End { epoch: 0 }));
    }

    #[test]
    fn resident_bytes_do_not_depend_on_clip_length() {
        let short = spawn(10, &calls());
        let long = spawn(10_000, &calls());
        assert_eq!(short.resident_bytes(), long.resident_bytes());
        assert_eq!(short.resident_bytes(), POOL_BUFFERS * FRAME_BYTES);

        // Play the long clip through: still the same pool.
        for i in 0..10_000u64 {
            assert_eq!(next_frame(&long).index, i);
        }
        assert!(matches!(next(&long), StreamMsg::End { .. }));
        assert_eq!(long.pool_allocations(), POOL_BUFFERS);
        assert_eq!(long.resident_bytes(), short.resident_bytes());
    }

    #[test]
    fn rejects_a_zero_frame_size() {
        let r = StreamDecoder::spawn(0, 720, 0, 0.0, false, |_, _| Err("unused".into()));
        assert!(r.is_err());
    }
}
