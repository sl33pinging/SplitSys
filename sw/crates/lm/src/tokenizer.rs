//! llama2.c BPE tokenizer (`tokenizer.bin` / `tok512.bin`), mirroring `run.c`
//! @ 350e04fe: `build_tokenizer`, `encode` (BOS, dummy-prefix " ", UTF-8 codepoint
//! lookup with byte fallback `+3`, greedy highest-score pair merging with a strict
//! `>` so the leftmost best pair wins), `decode` (strip a leading space after BOS,
//! `<0xHH>` byte pieces) and `safe_printf`.
//!
//! File format: int32 `max_token_length`, then per token: f32 score, int32 len,
//! `len` bytes. The vocabulary size is not in the file (it comes from the model).

use std::collections::HashMap;
use std::path::Path;

/// BOS token id (also the end-of-sequence delimiter in `generate`).
pub const BOS: u32 = 1;

#[derive(Clone, Debug)]
pub struct Tokenizer {
    vocab: Vec<Vec<u8>>,
    scores: Vec<f32>,
    lookup: HashMap<Vec<u8>, u32>,
    pub max_token_length: u32,
}

impl Tokenizer {
    pub fn load(path: &Path, vocab: usize) -> Result<Self, String> {
        let b = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Self::from_bytes(&b, vocab)
    }

    pub fn from_bytes(b: &[u8], vocab_size: usize) -> Result<Self, String> {
        let mut off = 0usize;
        let mut take = |n: usize, what: &str| -> Result<&[u8], String> {
            let s = b
                .get(off..off.saturating_add(n))
                .ok_or_else(|| format!("tokenizer truncated reading {what} at byte {off}"))?;
            off += n;
            Ok(s)
        };
        let rd4 = |s: &[u8]| [s[0], s[1], s[2], s[3]];
        let max_token_length = u32::from_le_bytes(rd4(take(4, "max_token_length")?));
        let mut vocab = Vec::with_capacity(vocab_size.min(1 << 20));
        let mut scores = Vec::with_capacity(vocab_size.min(1 << 20));
        for i in 0..vocab_size {
            let score = f32::from_le_bytes(rd4(take(4, "score")?));
            let len = i32::from_le_bytes(rd4(take(4, "len")?));
            let len =
                usize::try_from(len).map_err(|_| format!("token {i}: negative length {len}"))?;
            vocab.push(take(len, "piece")?.to_vec());
            scores.push(score);
        }
        // run.c bsearches a qsort-ed copy; with duplicate strings the hit is unspecified.
        // We keep the lowest id (tok512.bin has no duplicates; see tests).
        let mut lookup = HashMap::with_capacity(vocab_size);
        for (i, p) in vocab.iter().enumerate() {
            lookup.entry(p.clone()).or_insert(i as u32);
        }
        Ok(Self {
            vocab,
            scores,
            lookup,
            max_token_length,
        })
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }

    /// Raw piece bytes of a token (empty for an out-of-range id).
    pub fn piece(&self, tok: u32) -> &[u8] {
        self.vocab.get(tok as usize).map_or(&[], Vec::as_slice)
    }

    /// Number of distinct piece strings (equals `vocab_size()` iff no duplicates).
    pub fn distinct_pieces(&self) -> usize {
        self.lookup.len()
    }

    fn id(&self, s: &[u8]) -> Option<u32> {
        self.lookup.get(s).copied()
    }

    /// `run.c encode(t, text, bos, eos=0, ...)`.
    pub fn encode(&self, text: &str, bos: bool) -> Vec<u32> {
        let text = text.as_bytes();
        let mut tokens: Vec<u32> = Vec::new();
        if bos {
            tokens.push(BOS);
        }
        if !text.is_empty() {
            // run.c pushes str_lookup(" ") unconditionally (-1 if absent; never for real vocabs).
            if let Some(id) = self.id(b" ") {
                tokens.push(id);
            }
        }
        let mut buf: Vec<u8> = Vec::new();
        for (i, &c) in text.iter().enumerate() {
            if c & 0xC0 != 0x80 {
                buf.clear();
            }
            buf.push(c);
            let next = text.get(i + 1).copied().unwrap_or(0);
            if next & 0xC0 == 0x80 && buf.len() < 4 {
                continue;
            }
            match self.id(&buf) {
                Some(id) => tokens.push(id),
                None => tokens.extend(buf.iter().map(|&b| u32::from(b) + 3)),
            }
            buf.clear();
        }
        loop {
            let mut best_score = -1e10f32;
            let mut best: Option<(usize, u32)> = None;
            for i in 0..tokens.len().saturating_sub(1) {
                let mut s = self.piece(tokens[i]).to_vec();
                s.extend_from_slice(self.piece(tokens[i + 1]));
                if let Some(id) = self.id(&s) {
                    if self.scores[id as usize] > best_score {
                        best_score = self.scores[id as usize];
                        best = Some((i, id));
                    }
                }
            }
            let Some((idx, id)) = best else { break };
            tokens[idx] = id;
            tokens.remove(idx + 1);
        }
        tokens
    }

    /// `run.c decode(t, prev, tok)` as raw bytes.
    pub fn decode_bytes(&self, prev: u32, tok: u32) -> Vec<u8> {
        let mut p: &[u8] = self.piece(tok);
        if prev == BOS && p.first() == Some(&b' ') {
            p = &p[1..];
        }
        if let Some(b) = parse_byte_piece(p) {
            return vec![b];
        }
        p.to_vec()
    }

    /// `decode` as a String (lossy for pieces that are not valid UTF-8 on their own).
    pub fn decode(&self, prev: u32, tok: u32) -> String {
        String::from_utf8_lossy(&self.decode_bytes(prev, tok)).into_owned()
    }

    /// `decode` filtered like `run.c safe_printf`: single non-printable,
    /// non-space bytes are dropped.
    pub fn decode_safe(&self, prev: u32, tok: u32) -> Vec<u8> {
        let p = self.decode_bytes(prev, tok);
        if p.len() == 1 {
            let c = p[0];
            let printable = (0x20..=0x7e).contains(&c);
            let space = matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r');
            if !(printable || space) {
                return Vec::new();
            }
        }
        p
    }
}

/// `sscanf(piece, "<0x%02hhX>", &b) == 1`: "<0x" followed by 1-2 hex digits
/// (the trailing '>' is not checked by sscanf).
fn parse_byte_piece(p: &[u8]) -> Option<u8> {
    let rest = p.strip_prefix(b"<0x")?;
    let digits: Vec<u8> = rest
        .iter()
        .take(2)
        .take_while(|c| c.is_ascii_hexdigit())
        .copied()
        .collect();
    if digits.is_empty() {
        return None;
    }
    u8::from_str_radix(std::str::from_utf8(&digits).ok()?, 16).ok()
}
