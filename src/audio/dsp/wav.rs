//! A 16-bit stereo WAV writer, for the offline renders
//! (`probe --audio-out`, the dev server's `audio_render`).

use std::io::Write;

/// Writes interleaved stereo `frames` at `rate` as 16-bit PCM.
pub fn write(out: &mut impl Write, rate: u32, frames: &[f32]) -> std::io::Result<()> {
    let data_len = (frames.len() * 2) as u32;
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_len).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?;
    out.write_all(&2u16.to_le_bytes())?;
    out.write_all(&rate.to_le_bytes())?;
    out.write_all(&(rate * 4).to_le_bytes())?;
    out.write_all(&4u16.to_le_bytes())?;
    out.write_all(&16u16.to_le_bytes())?;
    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;
    for &v in frames {
        let s = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        out.write_all(&s.to_le_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_wav_header_names_its_size() {
        let mut bytes = Vec::new();
        super::write(&mut bytes, 48_000, &[0.0, 0.5, -0.5, 1.0]).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
        assert_eq!(bytes.len(), 44 + 8);
    }
}
