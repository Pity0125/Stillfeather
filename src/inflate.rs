//! DEFLATE 解壓縮（RFC 1951），給快速預覽讀 Office 文件（ZIP）用。只用標準函式庫。
//!
//! 支援三種區塊：不壓縮、固定 Huffman、動態 Huffman。解碼用 9 位元查表，較長的碼再逐位元比對。
//! 輸出超過 `limit` 位元組就停止並回報錯誤（防止壓縮炸彈）；任何不合法的輸入都回傳錯誤，不會 panic。

const FAST_BITS: u32 = 9;

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    count: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0, buf: 0, count: 0 }
    }

    /// 至少補到 `need` 個位元（資料結束時補 0，由呼叫端以 `overrun` 判斷）
    fn fill(&mut self, need: u32) {
        while self.count < need {
            let byte = self.data.get(self.pos).copied().unwrap_or(0);
            self.pos += 1;
            self.buf |= (byte as u64) << self.count;
            self.count += 8;
        }
    }

    fn overrun(&self) -> bool {
        // 讀到資料結尾之後（補上的 0）超過 8 個位元組，表示資料不完整
        self.pos > self.data.len() + 8
    }

    fn bits(&mut self, n: u32) -> Result<u32, String> {
        if n == 0 {
            return Ok(0);
        }
        self.fill(n);
        let v = (self.buf & ((1u64 << n) - 1)) as u32;
        self.buf >>= n;
        self.count -= n;
        if self.overrun() {
            return Err(tr!("壓縮資料不完整。", "The compressed data is incomplete.").into());
        }
        Ok(v)
    }

    fn peek(&mut self, n: u32) -> u32 {
        self.fill(n);
        (self.buf & ((1u64 << n) - 1)) as u32
    }

    fn consume(&mut self, n: u32) {
        self.buf >>= n;
        self.count -= n;
    }

    /// 捨棄到位元組邊界
    fn align(&mut self) {
        let drop = self.count % 8;
        self.consume(drop);
    }
}

/// 標準 Huffman 碼（canonical）：快速查表 + 依長度的計數與符號
struct Huffman {
    /// 低 9 位元的查表：(符號 << 4) | 長度；0 表示要走慢路徑
    fast: Vec<u16>,
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Huffman, String> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        // 檢查碼長是否過多（超額訂閱）
        let mut left: i32 = 1;
        for &c in counts.iter().skip(1) {
            left <<= 1;
            left -= c as i32;
            if left < 0 {
                return Err(tr!("Huffman 碼長不合法。", "Invalid Huffman code lengths.").into());
            }
        }
        let mut offs = [0u16; 16];
        for i in 1..15 {
            offs[i + 1] = offs[i] + counts[i];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (sym, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        // 快速表：把每個長度 ≤ 9 的碼（反轉位元，因為 DEFLATE 由低位元開始讀）填進去
        let mut fast = vec![0u16; 1 << FAST_BITS];
        let mut code: u32 = 0;
        let mut index = 0usize;
        for len in 1..=15u32 {
            for _ in 0..counts[len as usize] {
                if len <= FAST_BITS {
                    let rev = reverse(code, len);
                    let sym = symbols[index];
                    let step = 1u32 << len;
                    let mut j = rev;
                    while j < (1 << FAST_BITS) {
                        fast[j as usize] = (sym << 4) | len as u16;
                        j += step;
                    }
                }
                code += 1;
                index += 1;
            }
            code <<= 1;
        }
        Ok(Huffman { fast, counts, symbols })
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, String> {
        let look = bits.peek(FAST_BITS);
        let entry = self.fast[look as usize];
        if entry != 0 {
            bits.consume((entry & 0xF) as u32);
            if bits.overrun() {
                return Err(tr!("壓縮資料不完整。", "The compressed data is incomplete.").into());
            }
            return Ok(entry >> 4);
        }
        // 慢路徑：逐位元比對標準碼
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: i32 = 0;
        for len in 1..16 {
            code |= bits.bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(tr!("無效的 Huffman 碼。", "Invalid Huffman code.").into())
    }
}

fn reverse(mut code: u32, len: u32) -> u32 {
    let mut r = 0;
    for _ in 0..len {
        r = (r << 1) | (code & 1);
        code >>= 1;
    }
    r
}

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CODE_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

/// 解壓縮原始 DEFLATE 資料（沒有 zlib／gzip 標頭）。輸出超過 `limit` 位元組時回傳錯誤。
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    inflate_with_hint(data, limit, 0)
}

/// 同 [`inflate`]，`hint` 是預期的解壓後大小（例如 ZIP 記錄的大小），用來預先配置記憶體。
pub fn inflate_with_hint(data: &[u8], limit: usize, hint: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::with_capacity(hint.min(limit).min(64 * 1024 * 1024));
    run(data, limit, &mut out)?;
    Ok(out)
}

/// 只解壓前面 `max` 位元組（大型工作表只需要開頭的列）。回傳內容與是否已經完整。
pub fn inflate_prefix(data: &[u8], max: usize, hint: usize) -> Result<(Vec<u8>, bool), String> {
    let mut out: Vec<u8> = Vec::with_capacity(hint.min(max).min(64 * 1024 * 1024));
    match run(data, max, &mut out) {
        Ok(()) => Ok((out, true)),
        Err(e) if e == too_big() => Ok((out, false)),
        Err(e) => Err(e),
    }
}

fn too_big() -> String {
    tr!("解壓縮後超過大小上限。", "The decompressed data is larger than the size limit.").into()
}

fn run(data: &[u8], limit: usize, out: &mut Vec<u8>) -> Result<(), String> {
    let mut bits = Bits::new(data);
    let fixed = fixed_tables()?;
    loop {
        let last = bits.bits(1)?;
        match bits.bits(2)? {
            0 => {
                bits.align();
                let len = bits.bits(16)?;
                let nlen = bits.bits(16)?;
                if len != !nlen & 0xFFFF {
                    return Err(tr!("不壓縮區塊的長度不一致。", "The length of an uncompressed block doesn't match.").into());
                }
                if out.len() + len as usize > limit {
                    return Err(too_big());
                }
                // 位元緩衝區裡剩下的整數位元組要先用掉
                for _ in 0..len {
                    out.push(bits.bits(8)? as u8);
                }
            }
            1 => block(&mut bits, out, &fixed.0, &fixed.1, limit)?,
            2 => {
                let (lit, dist) = dynamic_tables(&mut bits)?;
                block(&mut bits, out, &lit, &dist, limit)?;
            }
            _ => return Err(tr!("無效的壓縮區塊類型。", "Invalid compressed block type.").into()),
        }
        if last == 1 {
            break;
        }
    }
    Ok(())
}

fn fixed_tables() -> Result<(Huffman, Huffman), String> {
    let mut lengths = [0u8; 288];
    for (i, l) in lengths.iter_mut().enumerate() {
        *l = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    Ok((Huffman::new(&lengths)?, Huffman::new(&[5u8; 30])?))
}

fn dynamic_tables(bits: &mut Bits) -> Result<(Huffman, Huffman), String> {
    let hlit = bits.bits(5)? as usize + 257;
    let hdist = bits.bits(5)? as usize + 1;
    let hclen = bits.bits(4)? as usize + 4;
    if hlit > 286 || hdist > 30 {
        return Err(tr!("動態 Huffman 表過大。", "The dynamic Huffman table is too large.").into());
    }
    let mut code_lengths = [0u8; 19];
    for &i in CODE_ORDER.iter().take(hclen) {
        code_lengths[i] = bits.bits(3)? as u8;
    }
    let code = Huffman::new(&code_lengths)?;
    let mut lengths = vec![0u8; hlit + hdist];
    let mut i = 0;
    while i < lengths.len() {
        let sym = code.decode(bits)?;
        match sym {
            0..=15 => {
                lengths[i] = sym as u8;
                i += 1;
            }
            16 => {
                let prev = *lengths.get(i.wrapping_sub(1)).ok_or(tr!("重複碼長缺少前一個值。", "A repeated code length has no previous value."))?;
                let n = 3 + bits.bits(2)? as usize;
                if i + n > lengths.len() {
                    return Err(tr!("碼長超出範圍。", "Code length out of range.").into());
                }
                lengths[i..i + n].fill(prev);
                i += n;
            }
            17 | 18 => {
                let n = if sym == 17 { 3 + bits.bits(3)? as usize } else { 11 + bits.bits(7)? as usize };
                if i + n > lengths.len() {
                    return Err(tr!("碼長超出範圍。", "Code length out of range.").into());
                }
                i += n; // 已經是 0
            }
            _ => return Err(tr!("無效的碼長符號。", "Invalid code length symbol.").into()),
        }
    }
    if lengths[256] == 0 {
        return Err(tr!("缺少區塊結尾碼。", "Missing end-of-block code.").into());
    }
    Ok((Huffman::new(&lengths[..hlit])?, Huffman::new(&lengths[hlit..])?))
}

fn block(bits: &mut Bits, out: &mut Vec<u8>, lit: &Huffman, dist: &Huffman, limit: usize) -> Result<(), String> {
    loop {
        let sym = lit.decode(bits)? as usize;
        if sym < 256 {
            if out.len() >= limit {
                return Err(too_big());
            }
            out.push(sym as u8);
        } else if sym == 256 {
            return Ok(());
        } else {
            let li = sym - 257;
            if li >= 29 {
                return Err(tr!("無效的長度碼。", "Invalid length code.").into());
            }
            let len = LEN_BASE[li] as usize + bits.bits(LEN_EXTRA[li] as u32)? as usize;
            let di = dist.decode(bits)? as usize;
            if di >= 30 {
                return Err(tr!("無效的距離碼。", "Invalid distance code.").into());
            }
            let d = DIST_BASE[di] as usize + bits.bits(DIST_EXTRA[di] as u32)? as usize;
            if d > out.len() {
                return Err(tr!("距離超出已解壓的資料。", "The distance goes past the decompressed data.").into());
            }
            if out.len() + len > limit {
                return Err(too_big());
            }
            let start = out.len() - d;
            if d >= len {
                out.extend_from_within(start..start + len);
            } else {
                // 重疊複製（例如重複同一個字元）
                for k in 0..len {
                    let b = out[start + k];
                    out.push(b);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 與 testdata/make_fixtures.py 相同的測試資料產生器
    pub fn sample_text() -> Vec<u8> {
        let mut s = String::new();
        for i in 0..400 {
            s.push_str(&format!("Stillfeather 預覽測試 第 {i} 行：The quick brown fox jumps over the lazy dog.\n"));
        }
        s.into_bytes()
    }

    pub fn sample_random(n: usize) -> Vec<u8> {
        let mut x: u32 = 12345;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1103515245).wrapping_add(12345) & 0x7FFF_FFFF;
                (x >> 16) as u8
            })
            .collect()
    }

    #[test]
    fn decodes_zlib_fixtures() {
        let text = sample_text();
        let random = sample_random(5000);
        let cases: [(&[u8], &[u8]); 6] = [
            (include_bytes!("../testdata/text-l9.deflate"), &text),
            (include_bytes!("../testdata/text-l1.deflate"), &text),
            (include_bytes!("../testdata/text-l0.deflate"), &text),
            (include_bytes!("../testdata/random-l9.deflate"), &random),
            (include_bytes!("../testdata/short-fixed.deflate"), b"abcabcabcabcabc hello hello"),
            (include_bytes!("../testdata/empty.deflate"), b""),
        ];
        for (i, (compressed, expected)) in cases.iter().enumerate() {
            let got = inflate(compressed, 1 << 20).unwrap_or_else(|e| panic!("case {i}: {e}"));
            assert_eq!(&got[..], *expected, "case {i}");
        }
    }

    #[test]
    fn prefix_stops_early() {
        let data = include_bytes!("../testdata/text-l9.deflate");
        let full = sample_text();
        let (head, complete) = inflate_prefix(data, 1000, 0).unwrap();
        assert!(!complete);
        assert!(head.len() <= 1000 && head.len() > 700, "{}", head.len());
        assert_eq!(head, full[..head.len()]);
        let (all, complete) = inflate_prefix(data, full.len(), 0).unwrap();
        assert!(complete);
        assert_eq!(all, full);
        assert!(inflate_prefix(b"\xff\xff\xff", 1000, 0).is_err());
    }

    #[test]
    fn limit_stops_bombs() {
        let text = sample_text();
        let compressed = include_bytes!("../testdata/text-l9.deflate");
        assert!(inflate(compressed, text.len() - 1).is_err());
        assert_eq!(inflate(compressed, text.len()).unwrap(), text);
    }

    #[test]
    fn garbage_never_panics() {
        let mut x: u32 = 7;
        for len in 0..300usize {
            let data: Vec<u8> = (0..len)
                .map(|_| {
                    x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                    (x >> 24) as u8
                })
                .collect();
            let _ = inflate(&data, 1 << 16);
        }
        // 截斷的正常資料
        let compressed = include_bytes!("../testdata/text-l9.deflate");
        for cut in [0, 1, 2, 10, compressed.len() / 2, compressed.len() - 1] {
            let _ = inflate(&compressed[..cut], 1 << 20);
        }
    }
}
