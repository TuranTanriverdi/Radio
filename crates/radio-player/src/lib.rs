pub const PLAYER_VERSION_V5: &str = "V5-2026-11-06-jitter-3s";

use thiserror::Error;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write, Cursor};
use std::sync::{Arc, atomic::{AtomicU32, Ordering}, Mutex};
use std::time::{Duration, Instant};
use std::fs::File;
use std::path::PathBuf;
use std::collections::VecDeque;

#[derive(Debug, Error)]
pub enum PlayerError {
    #[error("player error: {0}")]
    Playback(String),
    #[error("no station loaded")]
    NoStation,
    #[error("no audio device: {0}")]
    NoDevice(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerState { Stopped, Playing, Buffering, }

#[derive(Debug, Clone, Copy)]
pub enum PlayError { Unplayable, Unreachable, HeAac }
impl PlayError {
    pub fn msg(self) -> &'static str {
        match self {
            PlayError::Unplayable => "This station can't be played. Try another station.",
            PlayError::Unreachable => "This station can't be reached. It may be offline. Try another station.",
            PlayError::HeAac => "HE-AAC stream",
        }
    }
}

#[derive(Clone)]
pub struct VuMeter {
    level_bits: Arc<AtomicU32>,
    peak_bits: Arc<AtomicU32>,
}
impl VuMeter {
    pub fn new() -> Self { Self { level_bits: Arc::new(AtomicU32::new(0)), peak_bits: Arc::new(AtomicU32::new(0)), } }
    pub fn set_level(&self, level: f32) {
        let clamped = level.clamp(0.0, 1.0);
        self.level_bits.store(clamped.to_bits(), Ordering::Relaxed);
        let current_peak = f32::from_bits(self.peak_bits.load(Ordering::Relaxed));
        if clamped > current_peak { self.peak_bits.store(clamped.to_bits(), Ordering::Relaxed); }
    }
    pub fn level(&self) -> f32 { f32::from_bits(self.level_bits.load(Ordering::Relaxed)) }
    pub fn peak(&self) -> f32 { f32::from_bits(self.peak_bits.load(Ordering::Relaxed)) }
    pub fn reset_peak(&self) { self.peak_bits.store(0, Ordering::Relaxed); }
}

type SharedRecorder = Arc<Mutex<Option<Arc<Mutex<File>>>>>;

// V13 MEMORY FIX: SeekableHttpResponse was unbounded.
// Before: buffer: Vec<u8> grew forever - 128 kbps = ~57 MB/hour, 8h = ~460 MB.
// After: sliding window, capped at MAX_BUFFER (4 MiB). Old bytes are dropped
// once pos has moved far past them. Symphonia only seeks backwards a few KB
// for frame resync / probing, never back to hour 0, so a 4 MiB window is
// generous. Seek to a dropped position returns InvalidInput (same as EOF).
// Invariants: buffer holds absolute range [base, base+len), pos is absolute.
// trim() never drops the byte at pos or after it.
// Recording is preserved: only bytes freshly read from `inner` are written
// to the recorder, buffered re-reads are NOT re-recorded.
const SEEKABLE_MAX_BUFFER: usize = 4 * 1024 * 1024; // 4 MiB

struct SeekableHttpResponse {
    inner: Box<dyn Read + Send>,
    buffer: Vec<u8>,
    base: usize, // absolute offset of buffer[0]
    pos: usize,  // absolute read position
    recorder: SharedRecorder,
}
unsafe impl Send for SeekableHttpResponse {}
unsafe impl Sync for SeekableHttpResponse {}
impl SeekableHttpResponse {
    fn new(inner: Box<dyn Read + Send>, recorder: SharedRecorder) -> Self {
        Self { inner, buffer: Vec::with_capacity(128 * 1024), base: 0, pos: 0, recorder, }
    }
    fn buffered_end(&self) -> usize { self.base + self.buffer.len() }
    fn trim(&mut self) {
        if self.buffer.len() <= SEEKABLE_MAX_BUFFER { return; }
        let excess = self.buffer.len() - SEEKABLE_MAX_BUFFER;
        // Never drop pos itself: we may only drop bytes strictly before pos.
        let max_droppable = self.pos.saturating_sub(self.base);
        let to_drop = excess.min(max_droppable);
        if to_drop == 0 { return; }
        self.buffer.drain(..to_drop);
        self.base += to_drop;
    }
}
impl Read for SeekableHttpResponse {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        // Serve from window if pos is inside it
        if self.pos < self.buffered_end() {
            let idx = self.pos - self.base;
            let available = self.buffer.len() - idx;
            let to_copy = available.min(out.len());
            out[..to_copy].copy_from_slice(&self.buffer[idx..idx + to_copy]);
            self.pos += to_copy;
            return Ok(to_copy);
        }
        // pos == end: read fresh bytes from network
        let mut temp = vec![0u8; out.len().max(8192)];
        let n = self.inner.read(&mut temp)?;
        if n == 0 { return Ok(0); }
        if let Some(rec_arc) = &*self.recorder.lock().unwrap() {
            if let Ok(mut f) = rec_arc.lock() { let _ = f.write_all(&temp[..n]); let _ = f.flush(); }
        }
        self.buffer.extend_from_slice(&temp[..n]);
        self.trim();
        let to_copy = n.min(out.len());
        out[..to_copy].copy_from_slice(&temp[..to_copy]);
        self.pos += to_copy;
        Ok(to_copy)
    }
}
impl Seek for SeekableHttpResponse {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos: i64 = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(delta) => self.pos as i64 + delta,
            SeekFrom::End(_) => { return Err(io::Error::new(io::ErrorKind::Unsupported, "SeekFrom::End not supported")); }
        };
        if new_pos < 0 { return Err(io::Error::new(io::ErrorKind::InvalidInput, "Seek before start")); }
        let new_pos = new_pos as usize;
        // V13: seeking to bytes already dropped from the window cannot be satisfied
        if new_pos < self.base {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Seek before buffered window (dropped)"));
        }
        while new_pos > self.buffered_end() {
            let mut temp = [0u8; 8192];
            let n = self.inner.read(&mut temp)?;
            if n == 0 { break; }
            if let Some(rec_arc) = &*self.recorder.lock().unwrap() {
                if let Ok(mut f) = rec_arc.lock() { let _ = f.write_all(&temp[..n]); let _ = f.flush(); }
            }
            self.buffer.extend_from_slice(&temp[..n]);
            self.trim();
        }
        self.pos = new_pos.min(self.buffered_end());
        Ok(self.pos as u64)
    }
}

struct VuMeterSource<S> { inner: S, vu: VuMeter, count: usize, sum_sq: f32, }
impl<S> VuMeterSource<S> { fn new(inner: S, vu: VuMeter) -> Self { Self { inner, vu, count: 0, sum_sq: 0.0 } } }
impl<S> Iterator for VuMeterSource<S> where S: Source<Item = f32>, {
    type Item = f32;
    fn next(&mut self) -> Option<Self::Item> {
        let sample_opt = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.next()));
        let sample = match sample_opt {
            Ok(Some(s)) => s,
            Ok(None) => return None,
            Err(_) => {
                self.vu.set_level(0.0);
                return None;
            }
        };
        self.sum_sq += sample * sample;
        self.count += 1;
        if self.count >= 1024 {
            let rms = (self.sum_sq / self.count as f32).sqrt();
            let db = 20.0 * (rms.max(0.000001).log10());
            let level_db = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
            let level = level_db.powf(0.7);
            self.vu.set_level(level);
            self.count = 0;
            self.sum_sq = 0.0;
        }
        Some(sample)
    }
}
impl<S> Source for VuMeterSource<S> where S: Source<Item = f32>, {
    fn current_frame_len(&self) -> Option<usize> { std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.current_frame_len())).unwrap_or(None) }
    fn channels(&self) -> u16 { std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.channels())).unwrap_or(2) }
    fn sample_rate(&self) -> u32 { std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.sample_rate())).unwrap_or(44100) }
    fn total_duration(&self) -> Option<Duration> { std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.total_duration())).unwrap_or(None) }
}

#[derive(Debug, Clone, Copy)]
struct MyAdts { object_type: u8, sr_index: u8, sample_rate: u32, frame_len: usize }
const SR_TABLE: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

fn my_parse_adts(b: &[u8]) -> Option<MyAdts> {
    if b.len() < 7 || b[0] != 0xFF || (b[1] & 0xF6) != 0xF0 { return None; }
    let object_type = ((b[2] >> 6) & 3) + 1;
    let sr_index = (b[2] >> 2) & 0x0F;
    let frame_len = (((b[3] & 3) as usize) << 11) | ((b[4] as usize) << 3) | ((b[5] as usize) >> 5);
    if sr_index as usize >= SR_TABLE.len() || frame_len < 7 || frame_len > 8192 { return None; }
    // Additional sanity: profile must be 1-4 (AAC spec)
    if object_type == 0 || object_type > 4 { return None; }
    Some(MyAdts { object_type, sr_index, sample_rate: SR_TABLE[sr_index as usize], frame_len })
}

fn my_find_adts(buf: &[u8]) -> Option<MyAdts> {
    // Require two consecutive valid frames with same sr_index and object_type to avoid MP3 false positives
    for i in 0..buf.len().saturating_sub(7) {
        if let Some(h) = my_parse_adts(&buf[i..]) {
            let next = i + h.frame_len;
            if next + 7 <= buf.len() {
                if let Some(n) = my_parse_adts(&buf[next..]) {
                    if n.sr_index == h.sr_index && n.object_type == h.object_type {
                        return Some(h);
                    }
                }
            }
        }
    }
    None
}

fn check_head(content_type: &str, head: &[u8]) -> Result<Option<MyAdts>, PlayError> {
    if let Some(a) = my_find_adts(head) {
        println!("[player-core] ADTS validated: object_type={} sr_index={} sr={} len={}", a.object_type, a.sr_index, a.sample_rate, a.frame_len);
        if a.object_type == 2 {
            if a.sample_rate >= 32000 {
                return Ok(Some(a));
            } else {
                // LC at <=24 kHz is almost always HE-AAC with implicit SBR (ADTS can't signal 5/29, 2-bit profile field only 1-4)
                println!("[player-core] LC at {}Hz -> treating as HE-AAC (implicit SBR)", a.sample_rate);
                return Err(PlayError::HeAac);
            }
        } else if a.object_type == 5 || a.object_type == 29 || a.object_type == 3 {
            return Err(PlayError::HeAac);
        } else {
            if content_type.contains("aac") {
                return Err(PlayError::HeAac);
            }
            return Err(PlayError::Unplayable);
        }
    }
    // No validated ADTS pair found
    if content_type.contains("aac") || content_type.contains("aacp") || content_type.contains("mp4") {
        println!("[player-core] No validated ADTS but content-type {} suggests AAC -> HE-AAC fallback", content_type);
        return Err(PlayError::HeAac);
    }
    match content_type {
        "audio/mpeg" | "audio/mp3" | "audio/ogg" | "application/ogg" => Ok(None),
        _ => {
            if content_type.contains("audio/") {
                return Err(PlayError::HeAac);
            }
            Err(PlayError::Unplayable)
        },
    }
}

fn read_fill(r: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? { 0 => break, k => n += k }
    }
    Ok(n)
}

// ============================================================
// V11 - NOW PLAYING (ICY) - added, existing play() path untouched
// Player's play() sends Icy-MetaData: 0, so decoder never sees metadata.
// This helper opens a SEPARATE short connection with Icy-MetaData: 1,
// reads icy-metaint + one metadata block, returns StreamTitle='Artist - Song'.
// It never touches the audio decoder, so it cannot break playback.
// Returns None if station sends no ICY metadata (HLS / AAC / no title).
// ============================================================
pub fn fetch_icy_title(url: &str) -> Option<String> {
    let url_lower = url.to_lowercase();
    if url_lower.ends_with(".m3u8") || url_lower.contains(".m3u8?") { return None; }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(5))
        .build();
    let response = agent.get(url)
        .set("User-Agent", "ham-radio-app/0.1.0")
        .set("Accept", "*/*")
        .set("Icy-MetaData", "1")
        .call()
        .ok()?;
    if response.status() != 200 { return None; }
    let metaint: usize = response.header("icy-metaint")?.parse().ok()?;
    if metaint == 0 { return None; }
    let mut reader = response.into_reader();
    // Skip metaint bytes of audio
    let mut skipped = 0usize;
    let mut buf = vec![0u8; 8192];
    while skipped < metaint {
        let want = (metaint - skipped).min(buf.len());
        let n = reader.read(&mut buf[..want]).ok()?;
        if n == 0 { return None; }
        skipped += n;
    }
    // 1 byte = metadata length / 16
    let mut len_buf = [0u8; 1];
    if reader.read_exact(&mut len_buf).is_err() { return None; }
    let meta_len = len_buf[0] as usize * 16;
    if meta_len == 0 { return None; }
    let mut meta = vec![0u8; meta_len];
    if reader.read_exact(&mut meta).is_err() { return None; }
    let meta_str = String::from_utf8_lossy(&meta);
    // StreamTitle='Artist - Title';
    let title = meta_str.split("StreamTitle='").nth(1)?.split("';").next()?;
    let title = title.trim();
    if title.is_empty() { return None; }
    Some(title.to_string())
}

pub fn probe_url(url: &str) -> Result<(), PlayError> {
    let url_lower = url.to_lowercase();
    if url_lower.ends_with(".m3u8") || url_lower.contains(".m3u8?") { return Err(PlayError::Unplayable); }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(5))
        .build();
    let response = agent.get(url)
        .set("User-Agent", "ham-radio-app/0.1.0")
        .set("Accept", "*/*")
        .call()
        .map_err(|_| PlayError::Unreachable)?;
    if response.status() != 200 { return Err(PlayError::Unreachable); }
    let content_type = response.header("content-type").unwrap_or("unknown").to_lowercase();
    let mut head_buf = vec![0u8; 8192];
    let mut full_reader = response.into_reader();
    let n = read_fill(&mut full_reader, &mut head_buf).map_err(|_| PlayError::Unreachable)?;
    if n == 0 { return Err(PlayError::Unreachable); }
    head_buf.truncate(n);
    match check_head(&content_type, &head_buf) {
        Ok(_) => Ok(()),
        Err(PlayError::HeAac) => Ok(()),
        Err(e) => Err(e),
    }
}

// --- PURE RUST RUSTY_AAC FALLBACK V4 ---
// V3 fixes for "stutters after a few seconds" on radyotvonline:
// 1) OLD CODE STILL RUNNING: old log says "Primed: 12288 samples after 3 frames"
//    New code says "Primed: 44 chunks (~2.04s)". If you see old log, you didn't copy this file.
// 2) Bypass SeekableHttpResponse for AAC: it buffers ENTIRE stream forever (unbounded Vec).
//    Use direct Chain<Cursor, Box<Read>> - no unbounded cache, no extra copy.
// 3) Bigger jitter: prebuffer 2s (44 chunks), max 6s (128 chunks). 139ms was too shallow
//    for radyotvonline which sends exactly at bitrate + jitter. Burst at connect masks it
//    for first seconds, then buffer drains.
// 4) Offset-based leftover (no drain(..n) O(n^2) per frame).
// 5) Timing diagnostics: logs read_ms / decode_ms / channel depth every 5s so next log
//    proves whether it's network or decode.
mod rusty_aac_fallback {
    use super::*;
    use rusty_aac::AacDecoder;
    use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};

    const PREBUFFER_CHUNKS: usize = 66; // V4: 66 * 4096 / 2 / 44100 = 3.06s - radyotvonline needs 3s
    const MAX_CHUNKS: usize = 160;      // V4: ~7.4s cap

    pub struct RustyAacSource {
        rx: Receiver<Vec<f32>>,
        initial_queue: VecDeque<Vec<f32>>,
        cur: Vec<f32>,
        pos: usize,
        channels: u16,
        sample_rate: u32,
        finished: bool,
        count: usize,
        sum_sq: f32,
        vu: VuMeter,
        underrun_count: usize,
    }

    impl RustyAacSource {
        pub fn try_create(head: Vec<u8>, rest_reader: Box<dyn Read + Send>, recorder: SharedRecorder, vu: VuMeter) -> Result<Self, String> {
            // V12 FIX: AAC recording was 0 bytes - old code did `let _ = recorder;` and never wrote.
            // Now: decoder thread writes every raw ADTS chunk it reads to recorder (if Rec is on).
            // If Rec is pressed mid-song, recording starts from the next ADTS frame (each frame is self-contained).
            let rec_for_thread: SharedRecorder = recorder.clone();
            let (tx, rx) = sync_channel::<Vec<f32>>(MAX_CHUNKS);
            // Direct chain, no SeekableHttpResponse, no unbounded buffer
            let chained: Box<dyn Read + Send> = Box::new(Cursor::new(head).chain(rest_reader));
            let mut reader = BufReader::with_capacity(32 * 1024, chained);

            std::thread::spawn(move || {
                let mut decoder = AacDecoder::new();
                let mut leftover: Vec<u8> = Vec::with_capacity(32 * 1024);
                let mut leftover_pos: usize = 0; // offset, avoid drain O(n^2)
                let mut frames_decoded: usize = 0;
                let mut read_buf = vec![0u8; 16384];
                let mut last_log = Instant::now();
                let mut read_total_ms: u128 = 0;
                let mut decode_total_ms: u128 = 0;
                let mut fills: u128 = 0;

                loop {
                    // Compact leftover if offset grew large (single drain, amortized)
                    if leftover_pos > 8192 {
                        leftover.drain(..leftover_pos);
                        leftover_pos = 0;
                    }
                    // Decode ALL complete frames currently in leftover before reading network
                    let mut decoded_any = false;
                    let t_dec_start = Instant::now();
                    while leftover.len() - leftover_pos >= 7 {
                        match rusty_aac::parse_adts(&leftover[leftover_pos..]) {
                            Ok(header) => {
                                let frame_len = header.frame_length;
                                if frame_len == 0 || frame_len > 8192 {
                                    leftover_pos += 1;
                                    continue;
                                }
                                if leftover.len() - leftover_pos < frame_len {
                                    break; // need more bytes
                                }
                                let frame_bytes = &leftover[leftover_pos..leftover_pos + frame_len];
                                match decoder.decode(frame_bytes, None) {
                                    Ok(frame) => {
                                        if !frame.samples.is_empty() {
                                            if frames_decoded < 5 {
                                                println!("[rusty_aac] frame {}: {}Hz {}ch {} samples", frames_decoded, frame.sample_rate, frame.channels, frame.samples.len());
                                            }
                                            frames_decoded += 1;
                                            if tx.send(frame.samples.clone()).is_err() { return; }
                                            decoded_any = true;
                                        }
                                        leftover_pos += frame_len;
                                    },
                                    Err(e) => {
                                        if frames_decoded < 5 { eprintln!("[rusty_aac] decode error: {:?}", e); }
                                        leftover_pos += 1;
                                    }
                                }
                            },
                            Err(_) => { leftover_pos += 1; }
                        }
                    }
                    decode_total_ms += t_dec_start.elapsed().as_millis();
                    fills += 1;

                    // Periodic diagnostics every 5s
                    if last_log.elapsed() >= Duration::from_secs(5) {
                        let avg_read = if fills > 0 { read_total_ms / fills } else { 0 };
                        let avg_dec = if fills > 0 { decode_total_ms / fills } else { 0 };
                        println!("[rusty_aac] stats: frames={} leftover={}B avg_read={}ms avg_decode={}ms fills={}", frames_decoded, leftover.len() - leftover_pos, avg_read, avg_dec, fills);
                        last_log = Instant::now();
                        read_total_ms = 0; decode_total_ms = 0; fills = 0;
                    }

                    // Need more bytes
                    let t_read = Instant::now();
                    match reader.read(&mut read_buf) {
                        Ok(0) => {
                            if leftover.len() - leftover_pos == 0 { break; }
                            if !decoded_any { break; }
                        },
                        Ok(n) => {
                            read_total_ms += t_read.elapsed().as_millis();
                            // V12: record raw ADTS bytes (head bytes come through this same read, so they are recorded too)
                            if let Some(rec_arc) = &*rec_for_thread.lock().unwrap() {
                                if let Ok(mut f) = rec_arc.lock() { let _ = f.write_all(&read_buf[..n]); let _ = f.flush(); }
                            }
                            // If leftover is fully consumed, clear it to avoid growing Vec
                            if leftover_pos == leftover.len() {
                                leftover.clear(); leftover_pos = 0;
                            }
                            leftover.extend_from_slice(&read_buf[..n]);
                        },
                        Err(e) => {
                            eprintln!("[rusty_aac] read error: {}", e);
                            break;
                        }
                    }
                }
                println!("[rusty_aac] decoder thread finished, total frames {}", frames_decoded);
            });

            // Prime 2s
            let mut initial_queue: VecDeque<Vec<f32>> = VecDeque::new();
            let prime_start = Instant::now();
            while initial_queue.len() < PREBUFFER_CHUNKS && prime_start.elapsed() < Duration::from_secs(5) {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(chunk) => initial_queue.push_back(chunk),
                    Err(RecvTimeoutError::Timeout) => {
                        if initial_queue.len() >= 6 { break; }
                        continue;
                    },
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            if initial_queue.is_empty() {
                return Err("no audio decoded in prime phase".into());
            }
            // THIS LINE PROVES YOU RUN V3. Old code printed "Primed: 12288 samples after 3 frames"
            println!("[rusty_aac] Primed V5: {} chunks (~{:.2}s), 44100Hz 2ch", initial_queue.len(), initial_queue.len() as f32 * 4096.0 / 2.0 / 44100.0);
            Ok(Self {
                rx,
                initial_queue,
                cur: Vec::new(),
                pos: 0,
                channels: 2,
                sample_rate: 44100,
                finished: false,
                count: 0,
                sum_sq: 0.0,
                vu,
                underrun_count: 0,
            })
        }
    }

    impl Iterator for RustyAacSource {
        type Item = f32;
        fn next(&mut self) -> Option<Self::Item> {
            loop {
                if self.pos < self.cur.len() {
                    let s = self.cur[self.pos];
                    self.pos += 1;
                    self.sum_sq += s * s;
                    self.count += 1;
                    if self.count >= 1024 {
                        let rms = (self.sum_sq / self.count as f32).sqrt();
                        let db = 20.0 * (rms.max(0.000001).log10());
                        let level_db = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
                        self.vu.set_level(level_db.powf(0.7)); // V5 FIX: was max() which stuck VU at peak
                        self.count = 0;
                        self.sum_sq = 0.0;
                    }
                    return Some(s);
                }
                if let Some(chunk) = self.initial_queue.pop_front() {
                    self.cur = chunk; self.pos = 0; continue;
                }
                if self.finished { self.vu.set_level(0.0); return None; }
                match self.rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(chunk) => { self.cur = chunk; self.pos = 0; },
                    Err(RecvTimeoutError::Timeout) => {
                        self.underrun_count += 1;
                        if self.underrun_count % 10 == 0 {
                            eprintln!("[rusty_aac] UNDERRUN x{}: decoder slower than audio - check [rusty_aac] stats avg_read vs avg_decode above", self.underrun_count);
                        }
                        self.cur = vec![0.0; 2048]; self.pos = 0;
                    },
                    Err(RecvTimeoutError::Disconnected) => {
                        self.finished = true;
                        if self.cur.is_empty() { return None; }
                    }
                }
            }
        }
    }

    impl Source for RustyAacSource {
        fn current_frame_len(&self) -> Option<usize> { None }
        fn channels(&self) -> u16 { self.channels }
        fn sample_rate(&self) -> u32 { self.sample_rate }
        fn total_duration(&self) -> Option<Duration> { None }
    }
}

pub struct Player {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Option<Sink>,
    volume: f32,
    state: PlayerState,
    vu: VuMeter,
    recorder: SharedRecorder,
    recording_path: Option<PathBuf>,
    sleep_until: Option<Instant>,
}
impl Player {
    pub fn new() -> Result<Self, PlayerError> {
        println!("[player-core] VERSION {}", PLAYER_VERSION_V5);
        let (_stream, handle) = OutputStream::try_default().map_err(|e| PlayerError::NoDevice(e.to_string()))?;
        Ok(Self { _stream, handle, sink: None, volume: 0.7, state: PlayerState::Stopped, vu: VuMeter::new(), recorder: Arc::new(Mutex::new(None)), recording_path: None, sleep_until: None, })
    }
    pub fn play(&mut self, stream_url: &str) -> Result<(), PlayerError> {
        self.stop();
        println!("[player-core] Connecting to {} ...", stream_url);
        let sink = Sink::try_new(&self.handle).map_err(|e| PlayerError::Playback(e.to_string()))?;
        sink.set_volume(self.volume);
        let url_lower = stream_url.to_lowercase();
        if url_lower.ends_with(".m3u8") || url_lower.contains(".m3u8?") {
            return Err(PlayerError::Playback(PlayError::Unplayable.msg().to_string()));
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(5))
            .build();
        let response = agent.get(stream_url)
            .set("User-Agent", "ham-radio-app/0.1.0")
            .set("Accept", "*/*")
            .set("Icy-MetaData", "0")
            .call()
            .map_err(|e| {
                eprintln!("[player-core] ureq error: {:?}", e);
                PlayerError::Playback(PlayError::Unreachable.msg().to_string())
            })?;
        if response.status() != 200 {
            return Err(PlayerError::Playback(PlayError::Unreachable.msg().to_string()));
        }
        let content_type = response.header("content-type").unwrap_or("unknown").to_lowercase();
        println!("[player-core] Content-Type: {}", content_type);
        let mut head_buf = vec![0u8; 8192];
        let mut full_reader = response.into_reader();
        let n = read_fill(&mut full_reader, &mut head_buf).map_err(|e| {
            eprintln!("[player-core] read_fill error: {}", e);
            PlayerError::Playback(PlayError::Unreachable.msg().to_string())
        })?;
        if n == 0 { return Err(PlayerError::Playback(PlayError::Unreachable.msg().to_string())); }
        head_buf.truncate(n);
        println!("[player-core] First {} bytes: {:02X?}", head_buf.len().min(32), &head_buf[..head_buf.len().min(32)]);

        match check_head(&content_type, &head_buf) {
            Ok(_) => {
                println!("[player-core] Trying symphonia...");
                let head_clone = head_buf.clone();
                let chained: Box<dyn Read + Send> = Box::new(Cursor::new(head_clone).chain(full_reader));
                let seekable = SeekableHttpResponse::new(chained, self.recorder.clone());
                let buf_reader = BufReader::new(seekable);
                // Note: catch_unwind won't catch abort (0xcfffffff) - routing fix prevents ODTU from reaching here
                let decoder_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Decoder::new(buf_reader)));
                let decoder = match decoder_result {
                    Ok(Ok(dec)) => dec,
                    Ok(Err(e)) => {
                        eprintln!("[player-core] symphonia Decoder::new failed: {:?} - Unplayable", e);
                        return Err(PlayerError::Playback(PlayError::Unplayable.msg().to_string()));
                    },
                    Err(_) => {
                        return Err(PlayerError::Playback(PlayError::Unplayable.msg().to_string()));
                    }
                };
                let f32_source = decoder.convert_samples::<f32>();
                let vu_source = VuMeterSource::new(f32_source, self.vu.clone());
                sink.append(vu_source);
                self.sink = Some(sink);
                self.state = PlayerState::Playing;
                println!("[player-core] Playing via symphonia! Vol {:.0}%", self.volume * 100.0);
                Ok(())
            },
            Err(PlayError::HeAac) => {
                println!("[player-core] HE-AAC detected - trying rusty_aac pure Rust decoder");
                match rusty_aac_fallback::RustyAacSource::try_create(head_buf, Box::new(full_reader), self.recorder.clone(), self.vu.clone()) {
                    Ok(aac_source) => {
                        println!("[player-core] rusty_aac decoder active - playing HE-AAC {}Hz", aac_source.sample_rate());
                        sink.append(aac_source);
                        self.sink = Some(sink);
                        self.state = PlayerState::Playing;
                        Ok(())
                    },
                    Err(e) => {
                        eprintln!("[player-core] rusty_aac fallback failed: {} - Unplayable", e);
                        Err(PlayerError::Playback(PlayError::Unplayable.msg().to_string()))
                    }
                }
            },
            Err(pe) => {
                Err(PlayerError::Playback(pe.msg().to_string()))
            }
        }
    }
    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() { sink.stop(); }
        self.state = PlayerState::Stopped;
        self.vu.set_level(0.0);
    }
    pub fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 1.0);
        if let Some(sink) = &self.sink { sink.set_volume(self.volume); }
    }
    pub fn state(&mut self) -> PlayerState {
        if let Some(until) = self.sleep_until {
            if Instant::now() >= until {
                self.stop();
                self.sleep_until = None;
                return PlayerState::Stopped;
            }
        }
        if let Some(sink) = &self.sink { if sink.empty() && self.state == PlayerState::Playing { return PlayerState::Stopped; } }
        self.state.clone()
    }
    pub fn vu_level(&self) -> f32 { self.vu.level() }
    pub fn vu_peak(&self) -> f32 { self.vu.peak() }
    pub fn vu_meter(&self) -> VuMeter { self.vu.clone() }
    pub fn start_recording(&mut self, path: PathBuf) -> Result<(), PlayerError> {
        let file = File::create(&path).map_err(|e| PlayerError::Playback(format!("rec file: {}", e)))?;
        let arc_file = Arc::new(Mutex::new(file));
        *self.recorder.lock().unwrap() = Some(arc_file);
        self.recording_path = Some(path.clone());
        println!("[record] Started {:?}", path);
        Ok(())
    }
    pub fn stop_recording(&mut self) {
        // V12: flush before closing, otherwise last chunk can stay in OS buffer and file looks 0/short
        if let Some(rec_arc) = &*self.recorder.lock().unwrap() {
            if let Ok(mut f) = rec_arc.lock() { let _ = f.flush(); }
        }
        if let Some(path) = self.recording_path.take() { println!("[record] Stopped {:?}", path); }
        *self.recorder.lock().unwrap() = None;
    }
    pub fn is_recording(&self) -> bool { self.recorder.lock().unwrap().is_some() }
    pub fn set_sleep_timer(&mut self, minutes: u64) {
        self.sleep_until = Some(Instant::now() + Duration::from_secs(minutes * 60));
    }
    pub fn clear_sleep_timer(&mut self) { self.sleep_until = None; }
    pub fn sleep_remaining(&self) -> Option<Duration> {
        self.sleep_until.map(|until| { let now = Instant::now(); if until > now { until - now } else { Duration::from_secs(0) } })
    }
}
unsafe impl Send for Player {}
unsafe impl Sync for Player {}
