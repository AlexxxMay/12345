//! Минимальная реализация ZIP и Deflate (RFC 1950/1951) на стандартной библиотеке.
//!
//! Нужна для 3MF (OPC-пакет — ZIP-архив с XML внутри):
//! - чтение: методы «stored» (0) и «deflate» (8), включая динамические коды Хаффмана;
//! - запись: deflate с LZ77 (хеш-цепочки) и фиксированными кодами Хаффмана —
//!   простой и быстрый компрессор, для XML-сеток даёт сжатие в 4–8 раз.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// CRC-32
// ---------------------------------------------------------------------------

fn crc_table() -> &'static [u32; 256] {
    static T: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *v = c;
        }
        t
    })
}

pub fn crc32(data: &[u8]) -> u32 {
    let t = crc_table();
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = t[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

// ---------------------------------------------------------------------------
// Inflate
// ---------------------------------------------------------------------------

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u64,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, bit: 0, nbits: 0 }
    }
    #[inline]
    fn need(&mut self, n: u32) -> Result<(), String> {
        while self.nbits < n {
            let b = *self.data.get(self.pos).ok_or("deflate: неожиданный конец данных")?;
            self.pos += 1;
            self.bit |= (b as u64) << self.nbits;
            self.nbits += 8;
        }
        Ok(())
    }
    #[inline]
    fn bits(&mut self, n: u32) -> Result<u32, String> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let v = (self.bit & ((1u64 << n) - 1)) as u32;
        self.bit >>= n;
        self.nbits -= n;
        Ok(v)
    }
    fn align(&mut self) {
        let r = self.nbits % 8;
        self.bit >>= r;
        self.nbits -= r;
    }
}

/// Канонический код Хаффмана: таблица быстрого поиска на `FAST` бит + медленный путь.
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, String> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, br: &mut BitReader) -> Result<u16, String> {
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16 {
            code |= br.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err("deflate: неверный код Хаффмана".into())
    }
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

fn fixed_lengths() -> (Vec<u8>, Vec<u8>) {
    let mut ll = vec![0u8; 288];
    for (i, l) in ll.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    (ll, vec![5u8; 30])
}

/// Распаковка «сырого» потока deflate.
pub fn inflate(data: &[u8], size_hint: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::with_capacity(size_hint);
    let mut br = BitReader::new(data);
    loop {
        let last = br.bits(1)?;
        let kind = br.bits(2)?;
        match kind {
            0 => {
                br.align();
                let len = br.bits(16)? as usize;
                let nlen = br.bits(16)? as usize;
                if len != (!nlen & 0xFFFF) {
                    return Err("deflate: повреждён несжатый блок".into());
                }
                // Остаток буфера битов уже выровнен; читаем побайтно.
                for _ in 0..len {
                    out.push(br.bits(8)? as u8);
                }
            }
            1 | 2 => {
                let (lit, dist) = if kind == 1 {
                    let (ll, dl) = fixed_lengths();
                    (Huffman::new(&ll)?, Huffman::new(&dl)?)
                } else {
                    let hlit = br.bits(5)? as usize + 257;
                    let hdist = br.bits(5)? as usize + 1;
                    let hclen = br.bits(4)? as usize + 4;
                    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                    let mut cl = [0u8; 19];
                    for &o in ORDER.iter().take(hclen) {
                        cl[o] = br.bits(3)? as u8;
                    }
                    let clh = Huffman::new(&cl)?;
                    let mut lens = Vec::with_capacity(hlit + hdist);
                    while lens.len() < hlit + hdist {
                        let sym = clh.decode(&mut br)?;
                        match sym {
                            0..=15 => lens.push(sym as u8),
                            16 => {
                                let prev = *lens.last().ok_or("deflate: повтор без предыдущей длины")?;
                                for _ in 0..3 + br.bits(2)? {
                                    lens.push(prev);
                                }
                            }
                            17 => {
                                for _ in 0..3 + br.bits(3)? {
                                    lens.push(0);
                                }
                            }
                            _ => {
                                for _ in 0..11 + br.bits(7)? {
                                    lens.push(0);
                                }
                            }
                        }
                    }
                    if lens.len() > hlit + hdist {
                        return Err("deflate: неверные длины кодов".into());
                    }
                    (Huffman::new(&lens[..hlit])?, Huffman::new(&lens[hlit..])?)
                };
                loop {
                    let sym = lit.decode(&mut br)? as usize;
                    if sym < 256 {
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let li = sym - 257;
                        if li >= 29 {
                            return Err("deflate: неверная длина".into());
                        }
                        let len = LEN_BASE[li] as usize + br.bits(LEN_EXTRA[li] as u32)? as usize;
                        let di = dist.decode(&mut br)? as usize;
                        if di >= 30 {
                            return Err("deflate: неверное расстояние".into());
                        }
                        let d = DIST_BASE[di] as usize + br.bits(DIST_EXTRA[di] as u32)? as usize;
                        if d > out.len() {
                            return Err("deflate: ссылка за пределы данных".into());
                        }
                        let start = out.len() - d;
                        for k in 0..len {
                            let b = out[start + k];
                            out.push(b);
                        }
                    }
                }
            }
            _ => return Err("deflate: неверный тип блока".into()),
        }
        if last == 1 {
            break;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Deflate (фиксированные коды Хаффмана + LZ77)
// ---------------------------------------------------------------------------

struct BitWriter {
    out: Vec<u8>,
    bit: u64,
    nbits: u32,
}

impl BitWriter {
    #[inline]
    fn put(&mut self, v: u32, n: u32) {
        self.bit |= (v as u64) << self.nbits;
        self.nbits += n;
        while self.nbits >= 8 {
            self.out.push(self.bit as u8);
            self.bit >>= 8;
            self.nbits -= 8;
        }
    }
    /// Код Хаффмана пишется старшим битом вперёд.
    #[inline]
    fn put_rev(&mut self, code: u32, n: u32) {
        let mut r = 0;
        for i in 0..n {
            r |= ((code >> i) & 1) << (n - 1 - i);
        }
        self.put(r, n);
    }
    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            self.out.push(self.bit as u8);
        }
        self.out
    }
}

#[inline]
fn put_lit(w: &mut BitWriter, sym: u32) {
    match sym {
        0..=143 => w.put_rev(0x30 + sym, 8),
        144..=255 => w.put_rev(0x190 + sym - 144, 9),
        256..=279 => w.put_rev(sym - 256, 7),
        _ => w.put_rev(0xC0 + sym - 280, 8),
    }
}

fn put_match(w: &mut BitWriter, len: usize, dist: usize) {
    let li = LEN_BASE.iter().rposition(|&b| b as usize <= len).unwrap_or(0);
    put_lit(w, 257 + li as u32);
    w.put((len - LEN_BASE[li] as usize) as u32, LEN_EXTRA[li] as u32);
    let di = DIST_BASE.iter().rposition(|&b| b as usize <= dist).unwrap_or(0);
    w.put_rev(di as u32, 5);
    w.put((dist - DIST_BASE[di] as usize) as u32, DIST_EXTRA[di] as u32);
}

/// Сжатие в «сырой» поток deflate (один блок с фиксированными кодами).
pub fn deflate(data: &[u8]) -> Vec<u8> {
    const WIN: usize = 32768;
    const MAX_CHAIN: usize = 24;
    let mut w = BitWriter { out: Vec::with_capacity(data.len() / 3 + 16), bit: 0, nbits: 0 };
    w.put(1, 1); // последний блок
    w.put(1, 2); // фиксированные коды
    let n = data.len();
    let mut head: HashMap<u32, usize> = HashMap::new();
    let mut prev: Vec<usize> = vec![usize::MAX; n];
    let key = |i: usize| (data[i] as u32) | ((data[i + 1] as u32) << 8) | ((data[i + 2] as u32) << 16);
    let insert = |head: &mut HashMap<u32, usize>, prev: &mut Vec<usize>, i: usize| {
        if i + 2 < n {
            let k = key(i);
            if let Some(&p) = head.get(&k) {
                prev[i] = p;
            }
            head.insert(k, i);
        }
    };
    let mut i = 0;
    while i < n {
        let mut best_len = 0;
        let mut best_dist = 0;
        if i + 2 < n {
            if let Some(&cand) = head.get(&key(i)) {
                let mut c = cand;
                let mut chain = 0;
                while c != usize::MAX && i - c <= WIN && chain < MAX_CHAIN {
                    let max = (n - i).min(258);
                    let mut l = 0;
                    while l < max && data[c + l] == data[i + l] {
                        l += 1;
                    }
                    if l > best_len {
                        best_len = l;
                        best_dist = i - c;
                        if l == max {
                            break;
                        }
                    }
                    c = prev[c];
                    chain += 1;
                }
            }
        }
        if best_len >= 3 {
            put_match(&mut w, best_len, best_dist);
            for k in 0..best_len {
                insert(&mut head, &mut prev, i + k);
            }
            i += best_len;
        } else {
            put_lit(&mut w, data[i] as u32);
            insert(&mut head, &mut prev, i);
            i += 1;
        }
    }
    put_lit(&mut w, 256);
    w.finish()
}

// ---------------------------------------------------------------------------
// ZIP
// ---------------------------------------------------------------------------

fn u16le(d: &[u8], o: usize) -> Result<u16, String> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(|| "zip: повреждён архив".to_string())
}

fn u32le(d: &[u8], o: usize) -> Result<u32, String> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| "zip: повреждён архив".to_string())
}

/// Запись центрального каталога ZIP.
#[derive(Clone, Debug)]
pub struct ZipEntry {
    pub name: String,
    method: u16,
    comp_size: usize,
    size: usize,
    crc: u32,
    local_offset: usize,
}

/// Список файлов архива.
pub fn list(data: &[u8]) -> Result<Vec<ZipEntry>, String> {
    // Поиск конца центрального каталога (EOCD) с конца файла.
    if data.len() < 22 {
        return Err("zip: файл слишком мал".into());
    }
    let min = data.len().saturating_sub(22 + 65535);
    let mut eocd = None;
    let mut p = data.len() - 22;
    loop {
        if u32le(data, p)? == 0x0605_4b50 {
            eocd = Some(p);
            break;
        }
        if p == min {
            break;
        }
        p -= 1;
    }
    let eocd = eocd.ok_or("не ZIP-архив (нет каталога)")?;
    let count = u16le(data, eocd + 10)? as usize;
    let mut off = u32le(data, eocd + 16)? as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if u32le(data, off)? != 0x0201_4b50 {
            return Err("zip: повреждён центральный каталог".into());
        }
        let flags = u16le(data, off + 8)?;
        let method = u16le(data, off + 10)?;
        let crc = u32le(data, off + 16)?;
        let comp_size = u32le(data, off + 20)? as usize;
        let size = u32le(data, off + 24)? as usize;
        let nlen = u16le(data, off + 28)? as usize;
        let elen = u16le(data, off + 30)? as usize;
        let clen = u16le(data, off + 32)? as usize;
        let local_offset = u32le(data, off + 42)? as usize;
        if flags & 1 != 0 {
            return Err("zip: зашифрованные архивы не поддерживаются".into());
        }
        let name_b = data.get(off + 46..off + 46 + nlen).ok_or("zip: повреждено имя файла")?;
        let name = String::from_utf8_lossy(name_b).into_owned();
        out.push(ZipEntry { name, method, comp_size, size, crc, local_offset });
        off += 46 + nlen + elen + clen;
    }
    Ok(out)
}

/// Извлечение файла из архива.
pub fn extract(data: &[u8], e: &ZipEntry) -> Result<Vec<u8>, String> {
    let lo = e.local_offset;
    if u32le(data, lo)? != 0x0403_4b50 {
        return Err("zip: повреждён локальный заголовок".into());
    }
    let nlen = u16le(data, lo + 26)? as usize;
    let elen = u16le(data, lo + 28)? as usize;
    let start = lo + 30 + nlen + elen;
    let raw = data.get(start..start + e.comp_size).ok_or("zip: данные файла обрезаны")?;
    let out = match e.method {
        0 => raw.to_vec(),
        8 => inflate(raw, e.size)?,
        m => return Err(format!("zip: метод сжатия {m} не поддерживается")),
    };
    if out.len() != e.size || crc32(&out) != e.crc {
        return Err(format!("zip: контрольная сумма не совпала ({})", e.name));
    }
    Ok(out)
}

/// Создание ZIP-архива из пар (имя, содержимое). Все файлы сжимаются deflate.
pub fn create(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    // Дата/время DOS: 1 января 2026, 00:00.
    let (dos_time, dos_date) = (0u16, ((2026 - 1980) << 9 | 1 << 5 | 1) as u16);
    for (name, content) in files {
        let crc = crc32(content);
        let comp = deflate(content);
        let (method, body): (u16, &[u8]) = if comp.len() < content.len() { (8, &comp) } else { (0, content) };
        let offset = out.len() as u32;
        let nb = name.as_bytes();
        let mut hdr = Vec::with_capacity(30 + nb.len());
        hdr.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        hdr.extend_from_slice(&20u16.to_le_bytes()); // версия для распаковки
        hdr.extend_from_slice(&0x0800u16.to_le_bytes()); // флаги: имена в UTF-8
        hdr.extend_from_slice(&method.to_le_bytes());
        hdr.extend_from_slice(&dos_time.to_le_bytes());
        hdr.extend_from_slice(&dos_date.to_le_bytes());
        hdr.extend_from_slice(&crc.to_le_bytes());
        hdr.extend_from_slice(&(body.len() as u32).to_le_bytes());
        hdr.extend_from_slice(&(content.len() as u32).to_le_bytes());
        hdr.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        hdr.extend_from_slice(&0u16.to_le_bytes());
        hdr.extend_from_slice(nb);
        out.extend_from_slice(&hdr);
        out.extend_from_slice(body);

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // создано версией
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&dos_time.to_le_bytes());
        central.extend_from_slice(&dos_date.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(body.len() as u32).to_le_bytes());
        central.extend_from_slice(&(content.len() as u32).to_le_bytes());
        central.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 6]); // extra, comment, disk
        central.extend_from_slice(&0u16.to_le_bytes()); // внутренние атрибуты
        central.extend_from_slice(&0u32.to_le_bytes()); // внешние атрибуты
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(nb);
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn deflate_roundtrip() {
        let mut data = Vec::new();
        for i in 0..20000 {
            data.extend_from_slice(format!("<vertex x=\"{}\" y=\"{}\" z=\"0\"/>\n", i % 97, i % 13).as_bytes());
        }
        data.extend((0..3000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8));
        let c = deflate(&data);
        assert!(c.len() < data.len() / 4, "сжатие {} → {}", data.len(), c.len());
        assert_eq!(inflate(&c, data.len()).unwrap(), data);
        assert_eq!(inflate(&deflate(b""), 0).unwrap(), b"");
        assert_eq!(inflate(&deflate(b"ab"), 0).unwrap(), b"ab");
    }

    #[test]
    fn inflate_stored_and_dynamic() {
        // Несжатый блок: "hi".
        let stored = [0x01, 0x02, 0x00, 0xFD, 0xFF, b'h', b'i'];
        assert_eq!(inflate(&stored, 2).unwrap(), b"hi");
        // Поток с динамическими кодами Хаффмана (Python zlib, level 9, raw deflate).
        let expected: Vec<u8> = (0..300).flat_map(|i| format!("<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>\n", i * 7 % 113, i % 29, i % 5).into_bytes()).collect();
        let dynamic = include_bytes!("testdata/dyn.deflate");
        assert_eq!(inflate(dynamic, 0).unwrap(), expected);
        assert_eq!(crc32(&expected), 1_149_572_236);
    }

    #[test]
    fn zip_roundtrip() {
        let a = b"alpha alpha alpha alpha".to_vec();
        let b = vec![7u8; 5000];
        let z = create(&[("a.txt", &a), ("dir/b.bin", &b)]);
        let l = list(&z).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[1].name, "dir/b.bin");
        assert_eq!(extract(&z, &l[0]).unwrap(), a);
        assert_eq!(extract(&z, &l[1]).unwrap(), b);
    }
}
