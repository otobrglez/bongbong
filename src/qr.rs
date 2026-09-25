//! A QR code for the room's join link (docs/online-coop-prd.md §4.10),
//! encoded here rather than by a crate.
//!
//! The one thing the game ever needs to encode is a join URL of at most a
//! hundred characters, which is byte mode at error-correction level **L**
//! in one of versions 1 to 5. Those five versions hold their data in a
//! single Reed-Solomon block and carry a single alignment pattern, so the
//! whole encoder is the bit stream, one polynomial division over GF(256),
//! the module placement, the eight masks and their penalty scores - a few
//! hundred lines with no dependency, no C and nothing that could fail to
//! build for emscripten, iOS or Android, and nothing added to the room
//! server's graph.
//!
//! The result is a grid of modules, not pixels: [`draw`] paints it through
//! [`canvas::Canvas::fill_rect`](crate::canvas::Canvas::fill_rect) as whole
//! blocks at an integer scale, so it stays crisp at the game's resolution
//! and a test can render it with no window.

use crate::canvas::Canvas;
use crate::math::Color;

/// The mode the encoder uses: 8-bit byte, `0100`.
const MODE_BYTE: u32 = 0b0100;

/// Data and error-correction codewords per version at level L, versions 1
/// to 5. Every one of them is a single Reed-Solomon block, which is why
/// the encoder needs no interleaving.
const VERSIONS: [(usize, usize); 5] = [(19, 7), (34, 10), (55, 15), (80, 20), (108, 26)];

/// The most bytes this encoder can carry: version 5 at level L, less the
/// four mode bits and the eight length bits.
pub const MAX_BYTES: usize = 106;

/// The quiet zone the standard asks for, in modules on every side. Drawn
/// in the light colour, so the code reads against any background.
pub const QUIET_ZONE: i32 = 4;

/// Why a payload could not be encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLong {
    pub bytes: usize,
}

impl std::fmt::Display for TooLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} bytes is more than a version 5 code holds ({MAX_BYTES})", self.bytes)
    }
}

impl std::error::Error for TooLong {}

/// A finished code: `size` x `size` modules, row-major, `true` dark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Qr {
    size: usize,
    version: usize,
    mask: u8,
    modules: Vec<bool>,
}

impl Qr {
    /// Encode `payload` at the smallest version that holds it.
    pub fn encode(payload: &str) -> Result<Qr, TooLong> {
        let bytes = payload.as_bytes();
        let version = VERSIONS
            .iter()
            .position(|(data, _)| bytes.len() + 2 <= *data)
            .map(|i| i + 1)
            .ok_or(TooLong { bytes: bytes.len() })?;
        let (data_codewords, ecc_codewords) = VERSIONS[version - 1];
        let mut codewords = data_bits(bytes, data_codewords);
        codewords.extend(reed_solomon(&codewords, ecc_codewords));

        let size = 4 * version + 17;
        let mut grid = Grid::new(size);
        grid.function_patterns(version);
        grid.place_data(&codewords);
        let mask = grid.pick_mask();
        grid.apply_mask(mask);
        grid.format_info(mask);
        Ok(Qr { size, version, mask, modules: grid.modules })
    }

    /// Modules on a side, quiet zone excluded.
    pub fn size(&self) -> usize {
        self.size
    }

    /// The version, 1 to 5: `4 * version + 17` modules on a side.
    pub fn version(&self) -> usize {
        self.version
    }

    /// The data mask the encoder chose, 0 to 7.
    pub fn mask(&self) -> u8 {
        self.mask
    }

    /// Whether the module at (`x`, `y`) is dark. Anything outside the grid
    /// is light, which is what makes the quiet zone fall out of a loop over
    /// the padded square.
    pub fn dark(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x as usize >= self.size || y as usize >= self.size {
            return false;
        }
        self.modules[y as usize * self.size + x as usize]
    }

    /// Modules on a side with the quiet zone, the square [`draw`] fills.
    pub fn padded_size(&self) -> i32 {
        self.size as i32 + 2 * QUIET_ZONE
    }

    /// The largest whole-module scale that fits `box_px` pixels, at least
    /// one: the code is drawn in whole blocks or not at all, so a
    /// half-module edge never blurs a finder pattern.
    pub fn scale_for(&self, box_px: i32) -> i32 {
        (box_px / self.padded_size()).max(1)
    }
}

/// Paint `qr` with its top-left quiet-zone corner at (`x`, `y`), each
/// module a `scale` x `scale` block. The quiet zone is filled in `light`,
/// so the code carries its own margin over whatever is behind it.
pub fn draw(canvas: &mut impl Canvas, qr: &Qr, x: i32, y: i32, scale: i32, dark: Color, light: Color) {
    let side = qr.padded_size() * scale;
    canvas.fill_rect(x, y, side, side, light);
    for my in 0..qr.size as i32 {
        for mx in 0..qr.size as i32 {
            if qr.dark(mx, my) {
                canvas.fill_rect(x + (mx + QUIET_ZONE) * scale, y + (my + QUIET_ZONE) * scale, scale, scale, dark);
            }
        }
    }
}

/// The data codewords: mode, length, payload, terminator, then the pad
/// bytes the standard alternates until the block is full.
fn data_bits(payload: &[u8], data_codewords: usize) -> Vec<u8> {
    let mut bits = BitWriter::default();
    bits.push(MODE_BYTE, 4);
    // Versions 1 to 9 spell a byte-mode length in eight bits.
    bits.push(payload.len() as u32, 8);
    for &b in payload {
        bits.push(b as u32, 8);
    }
    let capacity = data_codewords * 8;
    let terminator = (capacity - bits.len()).min(4);
    bits.push(0, terminator);
    while bits.len() % 8 != 0 {
        bits.push(0, 1);
    }
    let mut out = bits.finish();
    for pad in [0xEC_u8, 0x11].into_iter().cycle() {
        if out.len() >= data_codewords {
            break;
        }
        out.push(pad);
    }
    out
}

/// The `count` error-correction codewords of `data`: the remainder of the
/// message polynomial divided by the generator of that degree, over
/// GF(256) with the QR field.
fn reed_solomon(data: &[u8], count: usize) -> Vec<u8> {
    let generator = generator_poly(count);
    let mut remainder = vec![0u8; count];
    for &byte in data {
        let factor = byte ^ remainder[0];
        remainder.rotate_left(1);
        remainder[count - 1] = 0;
        for (i, &g) in generator.iter().enumerate() {
            remainder[i] ^= gf_mul(g, factor);
        }
    }
    remainder
}

/// The generator polynomial of degree `count`, highest term first and the
/// leading 1 dropped: the product of (x - 2^i) for i in 0..count.
fn generator_poly(count: usize) -> Vec<u8> {
    let mut poly = vec![1u8];
    for i in 0..count {
        let root = gf_exp(i as u8);
        let mut next = vec![0u8; poly.len() + 1];
        for (j, &c) in poly.iter().enumerate() {
            next[j] ^= c;
            next[j + 1] ^= gf_mul(c, root);
        }
        poly = next;
    }
    poly[1..].to_vec()
}

/// GF(256) under the QR polynomial 0x11D, by repeated doubling. Small
/// enough that a table would only hide it.
fn gf_mul(a: u8, b: u8) -> u8 {
    let mut result = 0u8;
    let mut a = a;
    let mut b = b;
    while b != 0 {
        if b & 1 != 0 {
            result ^= a;
        }
        let high = a & 0x80;
        a <<= 1;
        if high != 0 {
            a ^= 0x1D;
        }
        b >>= 1;
    }
    result
}

/// 2^`power` in the field.
fn gf_exp(power: u8) -> u8 {
    let mut value = 1u8;
    for _ in 0..power {
        value = gf_mul(value, 2);
    }
    value
}

/// A most-significant-bit-first bit stream over whole bytes.
#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    fn push(&mut self, value: u32, count: usize) {
        for i in (0..count).rev() {
            if self.bits % 8 == 0 {
                self.bytes.push(0);
            }
            if value >> i & 1 != 0 {
                let byte = self.bits / 8;
                self.bytes[byte] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }

    fn len(&self) -> usize {
        self.bits
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// The grid under construction: the modules and which of them are function
/// patterns, which the data walk steps over and the mask leaves alone.
struct Grid {
    size: usize,
    modules: Vec<bool>,
    reserved: Vec<bool>,
}

impl Grid {
    fn new(size: usize) -> Grid {
        Grid { size, modules: vec![false; size * size], reserved: vec![false; size * size] }
    }

    fn index(&self, x: i32, y: i32) -> usize {
        y as usize * self.size + x as usize
    }

    fn set(&mut self, x: i32, y: i32, dark: bool) {
        let i = self.index(x, y);
        self.modules[i] = dark;
        self.reserved[i] = true;
    }

    fn is_reserved(&self, x: i32, y: i32) -> bool {
        self.reserved[self.index(x, y)]
    }

    /// Everything a decoder finds by shape: the three finders and their
    /// separators, the one alignment pattern of versions 2 and up, the two
    /// timing lines, the dark module, and the format-information strips
    /// held back for `format_info`.
    fn function_patterns(&mut self, version: usize) {
        let last = self.size as i32 - 7;
        for (ox, oy) in [(0, 0), (last, 0), (0, last)] {
            self.finder(ox, oy);
        }
        if version >= 2 {
            self.alignment(self.size as i32 - 7, self.size as i32 - 7);
        }
        for i in 8..self.size as i32 - 8 {
            let dark = i % 2 == 0;
            self.set(i, 6, dark);
            self.set(6, i, dark);
        }
        // The dark module, always set, sits just above the bottom-left
        // format strip.
        self.set(8, self.size as i32 - 8, true);
        self.reserve_format();
    }

    /// One 7 x 7 finder with the separator around it, clipped to the grid.
    /// A module's depth is how many rings in from the pattern's edge it
    /// sits: -1 is the separator, 0 the outer ring, 1 the light gap and 2
    /// the solid core.
    fn finder(&mut self, ox: i32, oy: i32) {
        for dy in -1..8 {
            for dx in -1..8 {
                let (x, y) = (ox + dx, oy + dy);
                if x < 0 || y < 0 || x >= self.size as i32 || y >= self.size as i32 {
                    continue;
                }
                let depth = dx.min(6 - dx).min(dy).min(6 - dy);
                self.set(x, y, depth >= 0 && depth != 1);
            }
        }
    }

    /// The 5 x 5 alignment pattern centred on (`cx`, `cy`).
    fn alignment(&mut self, cx: i32, cy: i32) {
        for dy in -2..3 {
            for dx in -2..3 {
                self.set(cx + dx, cy + dy, dx.abs().max(dy.abs()) != 1);
            }
        }
    }

    /// Hold the two format strips back so the data walk steps over them;
    /// `format_info` writes them once the mask is chosen.
    fn reserve_format(&mut self) {
        let size = self.size as i32;
        for i in 0..9 {
            if !self.is_reserved(i, 8) {
                self.set(i, 8, false);
            }
            if !self.is_reserved(8, i) {
                self.set(8, i, false);
            }
        }
        for i in 0..8 {
            if !self.is_reserved(size - 1 - i, 8) {
                self.set(size - 1 - i, 8, false);
            }
            if !self.is_reserved(8, size - 1 - i) {
                self.set(8, size - 1 - i, false);
            }
        }
    }

    /// The codewords into the free modules: two-module columns from the
    /// right, zigzagging up and down, skipping the vertical timing column.
    fn place_data(&mut self, codewords: &[u8]) {
        let size = self.size as i32;
        let mut bit = 0usize;
        let mut upward = true;
        let mut x = size - 1;
        while x > 0 {
            if x == 6 {
                x -= 1;
            }
            for step in 0..size {
                let y = if upward { size - 1 - step } else { step };
                for dx in 0..2 {
                    let cx = x - dx;
                    if self.is_reserved(cx, y) {
                        continue;
                    }
                    // Past the last codeword the remainder bits are zero.
                    let dark = codewords.get(bit / 8).is_some_and(|byte| byte >> (7 - bit % 8) & 1 != 0);
                    let i = self.index(cx, y);
                    self.modules[i] = dark;
                    bit += 1;
                }
            }
            upward = !upward;
            x -= 2;
        }
    }

    /// Flip the data modules by mask `pattern`.
    fn apply_mask(&mut self, pattern: u8) {
        for y in 0..self.size as i32 {
            for x in 0..self.size as i32 {
                if self.is_reserved(x, y) || !mask_bit(pattern, x, y) {
                    continue;
                }
                let i = self.index(x, y);
                self.modules[i] = !self.modules[i];
            }
        }
    }

    /// The mask with the lowest penalty, ties to the lower number.
    fn pick_mask(&mut self) -> u8 {
        let mut best = (u32::MAX, 0u8);
        for pattern in 0..8u8 {
            self.apply_mask(pattern);
            self.format_info(pattern);
            let score = self.penalty();
            self.apply_mask(pattern);
            if score < best.0 {
                best = (score, pattern);
            }
        }
        best.1
    }

    /// The fifteen format bits (level L and the mask, BCH-coded and
    /// XOR-masked), written in both of their places.
    fn format_info(&mut self, mask: u8) {
        // Level L is `01`; the mask follows in three bits.
        let data = 0b01_000 | mask as u32;
        let mut rest = data << 10;
        while 32 - rest.leading_zeros() >= 11 {
            rest ^= 0x537 << (32 - rest.leading_zeros() - 11);
        }
        let bits = ((data << 10) | rest) ^ 0x5412;
        let bit = |i: u32| bits >> i & 1 != 0;
        let size = self.size as i32;
        // The first copy wraps the top-left finder: the low bits down
        // column 8, the high bits left along row 8, skipping the two
        // timing modules in between.
        for i in 0..6 {
            self.set(8, i, bit(i as u32));
        }
        self.set(8, 7, bit(6));
        self.set(8, 8, bit(7));
        self.set(7, 8, bit(8));
        for i in 9..15 {
            self.set(14 - i, 8, bit(i as u32));
        }
        // The second copy: the low bits along row 8 from the right edge,
        // the high bits up column 8 from the bottom.
        for i in 0..8 {
            self.set(size - 1 - i, 8, bit(i as u32));
        }
        for i in 8..15 {
            self.set(8, size - 15 + i, bit(i as u32));
        }
        self.set(8, size - 8, true);
    }

    fn dark(&self, x: i32, y: i32) -> bool {
        self.modules[self.index(x, y)]
    }

    /// The standard's four penalty rules, summed.
    fn penalty(&self) -> u32 {
        let size = self.size as i32;
        let mut score = 0u32;
        // Rule 1: runs of five or more in a row or column.
        for along_row in [true, false] {
            for a in 0..size {
                let mut run = 0;
                let mut last = false;
                for b in 0..size {
                    let dark = if along_row { self.dark(b, a) } else { self.dark(a, b) };
                    if b > 0 && dark == last {
                        run += 1;
                        if run == 5 {
                            score += 3;
                        } else if run > 5 {
                            score += 1;
                        }
                    } else {
                        run = 1;
                    }
                    last = dark;
                }
            }
        }
        // Rule 2: every 2 x 2 block of one colour.
        for y in 0..size - 1 {
            for x in 0..size - 1 {
                let c = self.dark(x, y);
                if self.dark(x + 1, y) == c && self.dark(x, y + 1) == c && self.dark(x + 1, y + 1) == c {
                    score += 3;
                }
            }
        }
        // Rule 3: the finder-lookalike, either way round, in a row or column.
        const FINDER: [bool; 11] = [true, false, true, true, true, false, true, false, false, false, false];
        for along_row in [true, false] {
            for a in 0..size {
                for b in 0..=size - 11 {
                    let at = |i: i32| if along_row { self.dark(b + i, a) } else { self.dark(a, b + i) };
                    let forward = (0..11).all(|i| at(i) == FINDER[i as usize]);
                    let backward = (0..11).all(|i| at(i) == FINDER[10 - i as usize]);
                    if forward || backward {
                        score += 40;
                    }
                }
            }
        }
        // Rule 4: how far the dark share is from half.
        let dark = self.modules.iter().filter(|&&m| m).count();
        let percent = dark * 100 / self.modules.len();
        let away = percent.abs_diff(50);
        score += (away / 5) as u32 * 10;
        score
    }
}

/// Whether mask `pattern` flips the module at (`x`, `y`).
fn mask_bit(pattern: u8, x: i32, y: i32) -> bool {
    let (i, j) = (y as u32, x as u32);
    match pattern {
        0 => (i + j) % 2 == 0,
        1 => i % 2 == 0,
        2 => j % 3 == 0,
        3 => (i + j) % 3 == 0,
        4 => (i / 2 + j / 3) % 2 == 0,
        5 => (i * j) % 2 + (i * j) % 3 == 0,
        6 => ((i * j) % 2 + (i * j) % 3) % 2 == 0,
        _ => ((i + j) % 2 + (i * j) % 3) % 2 == 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::CpuCanvas;

    /// Read a finished code back: the format bits, the mask, the data
    /// modules in placement order, the payload and the Reed-Solomon
    /// syndromes. Everything the encoder wrote, checked from the grid
    /// rather than from the encoder's own intermediates.
    struct Decoded {
        mask: u8,
        payload: String,
        /// The syndromes at the spec's roots: all zero for a valid block.
        syndromes: Vec<u8>,
    }

    fn decode(qr: &Qr) -> Decoded {
        // The format strip around the top-left finder, most significant
        // bit first, un-XORed.
        let mut bits = 0u32;
        let read = |x: i32, y: i32| qr.dark(x, y) as u32;
        for i in 0..6 {
            bits |= read(8, i) << i;
        }
        bits |= read(8, 7) << 6;
        bits |= read(8, 8) << 7;
        bits |= read(7, 8) << 8;
        for i in 9..15 {
            bits |= read(14 - i, 8) << i;
        }
        let format = bits ^ 0x5412;
        assert_eq!(format >> 13, 0b01, "the level bits say L");
        let mask = (format >> 10 & 0b111) as u8;

        // The function patterns of this version, to tell data modules
        // from the rest and to walk them in placement order.
        let mut grid = Grid::new(qr.size());
        grid.function_patterns(qr.version());
        let size = qr.size() as i32;
        let mut bitstream: Vec<bool> = Vec::new();
        let mut upward = true;
        let mut x = size - 1;
        while x > 0 {
            if x == 6 {
                x -= 1;
            }
            for step in 0..size {
                let y = if upward { size - 1 - step } else { step };
                for dx in 0..2 {
                    let cx = x - dx;
                    if grid.is_reserved(cx, y) {
                        continue;
                    }
                    bitstream.push(qr.dark(cx, y) ^ mask_bit(mask, cx, y));
                }
            }
            upward = !upward;
            x -= 2;
        }

        let codewords: Vec<u8> = bitstream
            .chunks(8)
            .filter(|c| c.len() == 8)
            .map(|c| c.iter().fold(0u8, |acc, &b| acc << 1 | b as u8))
            .collect();
        let (data, ecc) = VERSIONS[qr.version() - 1];
        let block = &codewords[..data + ecc];
        let syndromes = (0..ecc)
            .map(|i| {
                let root = gf_exp(i as u8);
                block.iter().fold(0u8, |acc, &c| gf_mul(acc, root) ^ c)
            })
            .collect();

        assert_eq!(block[0] >> 4, MODE_BYTE as u8, "byte mode");
        let len = ((block[0] & 0x0F) << 4 | block[1] >> 4) as usize;
        let payload: Vec<u8> = (0..len).map(|i| block[1 + i] << 4 | block[2 + i] >> 4).collect();
        Decoded { mask, payload: String::from_utf8(payload).expect("ASCII"), syndromes }
    }

    const LINK: &str = "https://bongbong.io/j/AK7QX";

    #[test]
    fn a_join_link_encodes_at_the_smallest_version_that_holds_it() {
        let qr = Qr::encode(LINK).expect("27 bytes fit");
        assert_eq!(qr.version(), 2, "27 bytes is past version 1's 17");
        assert_eq!(qr.size(), 25, "4 * version + 17");
        assert_eq!(qr.padded_size(), 25 + 2 * QUIET_ZONE);
        // The local override's longer link needs one version more.
        let long = Qr::encode(&format!("{LINK}?rooms=ws://127.0.0.1:4848")).expect("53 bytes fit");
        assert_eq!(long.version(), 3);
        assert_eq!(Qr::encode(&"x".repeat(17)).unwrap().version(), 1);
        assert_eq!(Qr::encode(&"x".repeat(18)).unwrap().version(), 2);
        assert_eq!(Qr::encode(&"x".repeat(MAX_BYTES)).unwrap().version(), 5);
        assert_eq!(Qr::encode(&"x".repeat(MAX_BYTES + 1)), Err(TooLong { bytes: MAX_BYTES + 1 }));
    }

    #[test]
    fn the_function_patterns_are_where_a_scanner_looks_for_them() {
        let qr = Qr::encode(LINK).expect("encodes");
        let last = qr.size() as i32 - 7;
        for (ox, oy) in [(0, 0), (last, 0), (0, last)] {
            // The outer ring dark, the gap light, the 3 x 3 core dark.
            for i in 0..7 {
                assert!(qr.dark(ox + i, oy), "finder top edge at {ox},{oy}");
                assert!(qr.dark(ox + i, oy + 6), "finder bottom edge at {ox},{oy}");
                assert!(qr.dark(ox, oy + i), "finder left edge at {ox},{oy}");
                assert!(qr.dark(ox + 6, oy + i), "finder right edge at {ox},{oy}");
            }
            for i in 1..6 {
                assert!(!qr.dark(ox + i, oy + 1), "finder gap at {ox},{oy}");
                assert!(!qr.dark(ox + 1, oy + i), "finder gap at {ox},{oy}");
            }
            for dy in 2..5 {
                for dx in 2..5 {
                    assert!(qr.dark(ox + dx, oy + dy), "finder core at {ox},{oy}");
                }
            }
        }
        // The timing lines alternate, starting and ending dark.
        for i in 8..qr.size() as i32 - 8 {
            assert_eq!(qr.dark(i, 6), i % 2 == 0, "horizontal timing at {i}");
            assert_eq!(qr.dark(6, i), i % 2 == 0, "vertical timing at {i}");
        }
        // The one alignment pattern of a version 2 code, and the dark module.
        let c = qr.size() as i32 - 7;
        assert!(qr.dark(c, c) && !qr.dark(c + 1, c) && qr.dark(c + 2, c));
        assert!(qr.dark(8, qr.size() as i32 - 8), "the dark module");
        // The quiet zone: everything outside the grid reads light.
        for i in -QUIET_ZONE..qr.size() as i32 + QUIET_ZONE {
            for edge in [-1, qr.size() as i32] {
                assert!(!qr.dark(i, edge) && !qr.dark(edge, i));
            }
        }
    }

    /// The strong check: read the grid back the way a scanner would - the
    /// format bits, the mask, the data walk - and confirm the payload comes
    /// out and every Reed-Solomon syndrome is zero.
    #[test]
    fn the_grid_reads_back_as_the_payload_it_was_made_from() {
        for payload in [LINK, "https://bongbong.io/j/CDFGH?rooms=ws://127.0.0.1:4848", "A", &"z".repeat(MAX_BYTES)] {
            let qr = Qr::encode(payload).expect("fits");
            let decoded = decode(&qr);
            assert_eq!(decoded.mask, qr.mask(), "the format bits name the mask that was applied");
            assert_eq!(decoded.payload, payload);
            assert!(decoded.syndromes.iter().all(|&s| s == 0), "{payload}: {:?}", decoded.syndromes);
        }
    }

    /// A pinned fixture: the module grid of one link, as a hash. It moves
    /// only if the encoder does, which for a finished standard means a bug
    /// either way.
    #[test]
    fn a_known_link_keeps_its_modules() {
        let qr = Qr::encode(LINK).expect("encodes");
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for &m in &qr.modules {
            hash ^= m as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        assert_eq!(qr.mask(), 7);
        assert_eq!(hash, 0x839d_a57f_80db_6745, "the modules of {LINK} moved");
    }

    #[test]
    fn drawing_fills_whole_blocks_with_the_quiet_zone_around_them() {
        let qr = Qr::encode(LINK).expect("encodes");
        let scale = qr.scale_for(148);
        assert_eq!(scale, 4, "33 padded modules at 4 px is 132, 5 would overflow 148");
        let side = (qr.padded_size() * scale) as usize;
        let mut canvas = CpuCanvas::blank(side + 8, side + 8);
        draw(&mut canvas, &qr, 4, 4, scale, Color::BLACK, Color::WHITE);
        let at = |x: usize, y: usize| canvas.pixel(x, y);
        // The quiet zone is light all the way round the drawn square.
        for i in 0..side {
            assert_eq!(at(4 + i, 4), Color::WHITE, "top quiet zone");
            assert_eq!(at(4, 4 + i), Color::WHITE, "left quiet zone");
        }
        // The top-left finder's corner is one whole block of dark.
        let ox = 4 + QUIET_ZONE as usize * scale as usize;
        for dy in 0..scale as usize {
            for dx in 0..scale as usize {
                assert_eq!(at(ox + dx, ox + dy), Color::BLACK, "the finder's corner block");
            }
        }
        // Every drawn pixel is one of the two colours: whole blocks, no blend.
        assert!((0..side).all(|y| (0..side).all(|x| {
            let c = at(4 + x, 4 + y);
            c == Color::BLACK || c == Color::WHITE
        })));
    }
}

