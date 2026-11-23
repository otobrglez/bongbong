//! Sample data: a decoded sound held in memory (`Pcm`), and the Ogg Vorbis
//! decoder every shipped file goes through (`lewton`, pure Rust), so a
//! headless build decodes the same bytes the window does.

use std::sync::Arc;

/// One decoded sound: 16-bit samples, interleaved when stereo.
#[derive(Debug)]
pub struct Pcm {
    /// Frames a second.
    pub rate: u32,
    /// 1 or 2.
    pub channels: u16,
    /// `frames * channels` samples.
    pub data: Vec<i16>,
}

impl Pcm {
    pub fn new(rate: u32, channels: u16, data: Vec<i16>) -> Arc<Pcm> {
        assert!(channels == 1 || channels == 2, "a sound is mono or stereo");
        Arc::new(Pcm { rate, channels, data })
    }

    /// How many frames it holds.
    pub fn frames(&self) -> usize {
        self.data.len() / self.channels as usize
    }

    /// Its length in seconds.
    pub fn seconds(&self) -> f32 {
        self.frames() as f32 / self.rate.max(1) as f32
    }

    /// Frame `i` as left and right in -1..1; a mono sound gives the same
    /// value twice, and a frame outside the sound is silence.
    #[inline]
    pub fn frame(&self, i: isize) -> (f32, f32) {
        if i < 0 || i as usize >= self.frames() {
            return (0.0, 0.0);
        }
        let i = i as usize;
        const SCALE: f32 = 1.0 / 32768.0;
        if self.channels == 1 {
            let v = self.data[i] as f32 * SCALE;
            (v, v)
        } else {
            (self.data[2 * i] as f32 * SCALE, self.data[2 * i + 1] as f32 * SCALE)
        }
    }

    /// The loudest sample, as a share of full scale.
    pub fn peak(&self) -> f32 {
        self.data.iter().map(|&s| (s as i32).unsigned_abs()).max().unwrap_or(0) as f32 / 32768.0
    }

    /// The root mean square level, as a share of full scale.
    pub fn rms(&self) -> f32 {
        if self.data.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.data.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum();
        (sum / self.data.len() as f64).sqrt() as f32
    }
}

/// Decodes a whole Ogg Vorbis file.
pub fn ogg(bytes: &[u8]) -> Result<Arc<Pcm>, String> {
    let mut reader = lewton::inside_ogg::OggStreamReader::new(std::io::Cursor::new(bytes)).map_err(|e| format!("ogg: {e}"))?;
    let channels = reader.ident_hdr.audio_channels as u16;
    let rate = reader.ident_hdr.audio_sample_rate;
    if channels != 1 && channels != 2 {
        return Err(format!("ogg: {channels} channels; a sound is mono or stereo"));
    }
    let mut data = Vec::new();
    while let Some(packet) = reader.read_dec_packet_itl().map_err(|e| format!("ogg: {e}"))? {
        data.extend_from_slice(&packet);
    }
    Ok(Pcm::new(rate, channels, data))
}
