const HOLD_BACK_CAP: usize = 128 * 1024;
const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

#[derive(Clone, Copy, PartialEq, Default)]
enum State {
    #[default]
    Ground,
    Escape,
    EscapeIntermediate,
    Csi,
    StringBody,
    Utf8 {
        remaining: u8,
    },
}

#[derive(Default)]
pub struct HoldBack {
    state: State,
    pending: Vec<u8>,
}

impl HoldBack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<u8> {
        let mut ready = Vec::with_capacity(self.pending.len() + chunk.len());
        let mut rest = chunk;
        while !rest.is_empty() {
            if self.state == State::Ground {
                let plain = rest
                    .iter()
                    .position(|&byte| byte == ESC || byte >= 0x80)
                    .unwrap_or(rest.len());
                ready.extend_from_slice(&rest[..plain]);
                rest = &rest[plain..];
                if rest.is_empty() {
                    break;
                }
            }
            self.consume(rest[0], &mut ready);
            rest = &rest[1..];
        }
        ready
    }

    pub fn flush(&mut self) -> Vec<u8> {
        self.state = State::Ground;
        std::mem::take(&mut self.pending)
    }

    fn consume(&mut self, byte: u8, ready: &mut Vec<u8>) {
        if matches!(self.state, State::Utf8 { .. }) && !is_utf8_continuation(byte) {
            ready.append(&mut self.pending);
            self.state = State::Ground;
        }
        let next = transition(self.state, byte);
        if self.state == State::Ground && next == State::Ground {
            ready.push(byte);
            return;
        }
        self.pending.push(byte);
        self.state = next;
        if self.state == State::Ground || self.pending.len() >= HOLD_BACK_CAP {
            ready.append(&mut self.pending);
            self.state = State::Ground;
        }
    }
}

fn transition(state: State, byte: u8) -> State {
    match state {
        State::Ground => from_ground(byte),
        State::Escape => from_escape(byte),
        State::EscapeIntermediate => match byte {
            0x30..=0x7e => State::Ground,
            ESC => State::Escape,
            _ => State::EscapeIntermediate,
        },
        State::Csi => match byte {
            0x40..=0x7e => State::Ground,
            ESC => State::Escape,
            _ => State::Csi,
        },
        State::StringBody => match byte {
            BEL => State::Ground,
            ESC => State::Escape,
            _ => State::StringBody,
        },
        State::Utf8 { remaining: 1 } => State::Ground,
        State::Utf8 { remaining } => State::Utf8 {
            remaining: remaining - 1,
        },
    }
}

fn from_ground(byte: u8) -> State {
    match byte {
        ESC => State::Escape,
        0xc2..=0xdf => State::Utf8 { remaining: 1 },
        0xe0..=0xef => State::Utf8 { remaining: 2 },
        0xf0..=0xf4 => State::Utf8 { remaining: 3 },
        _ => State::Ground,
    }
}

fn from_escape(byte: u8) -> State {
    match byte {
        b'[' => State::Csi,
        b']' | b'P' | b'X' | b'^' | b'_' => State::StringBody,
        0x20..=0x2f => State::EscapeIntermediate,
        0x30..=0x7e => State::Ground,
        _ => State::Escape,
    }
}

fn is_utf8_continuation(byte: u8) -> bool {
    (0x80..=0xbf).contains(&byte)
}

#[cfg(test)]
mod tests {
    use super::{HoldBack, HOLD_BACK_CAP};

    const SGR: &[u8] = b"plain \x1b[31mred\x1b[0m \x1b[1;38;5;196mloud\x1b[m tail";
    const CURSOR_MOVES: &[u8] = b"\x1b[2J\x1b[H\x1b[10;20Hmoved\x1b[A\x1b[2B\x1b[5C\x1b[D";
    const OSC_TITLE_BEL: &[u8] = b"\x1b]0;tyba \xc3\xa9 title\x07after";
    const OSC_LINK_ST: &[u8] = b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\";
    const DCS_ST: &[u8] = b"\x1bP1$r0;1m\x1b\\";
    const CHARSET_ESCAPE: &[u8] = b"\x1b(B\x1b)0ascii";
    const UTF8_TEXT: &[u8] = "caf\u{e9} — a\u{e7}\u{e3}o 世界 🚀🔥 fim".as_bytes();

    fn osc52_clipboard() -> Vec<u8> {
        let mut seq = b"\x1b]52;c;".to_vec();
        seq.resize(seq.len() + 4096, b'Q');
        seq.push(0x07);
        seq
    }

    fn corpus() -> Vec<Vec<u8>> {
        vec![
            SGR.to_vec(),
            CURSOR_MOVES.to_vec(),
            OSC_TITLE_BEL.to_vec(),
            OSC_LINK_ST.to_vec(),
            DCS_ST.to_vec(),
            CHARSET_ESCAPE.to_vec(),
            UTF8_TEXT.to_vec(),
            osc52_clipboard(),
        ]
    }

    fn mixed_stream() -> Vec<u8> {
        let mut stream = Vec::new();
        for part in corpus() {
            stream.extend_from_slice("entre 🌊 ".as_bytes());
            stream.extend_from_slice(&part);
        }
        stream
    }

    fn firehose(lines: usize) -> Vec<u8> {
        let mut stream = Vec::new();
        for i in 0..lines {
            stream.extend_from_slice(
                format!(
                    "\x1b[1;3{}m[{:06}]\x1b[0m \x1b[38;2;{};{};200mtruecolor bloco\x1b[0m \x1b]0;t{}\x07 世界🚀🔥 \x1b[38;5;{}m################\x1b[0m\r\n",
                    i % 7 + 1,
                    i,
                    i % 255,
                    i % 200,
                    i,
                    i % 256
                )
                .as_bytes(),
            );
        }
        stream
    }

    fn ends_on_complete_sequences(chunk: &[u8]) -> bool {
        let mut i = 0;
        while i < chunk.len() {
            match sequence_end(chunk, i) {
                Some(end) => i = end,
                None => return false,
            }
        }
        true
    }

    fn sequence_end(bytes: &[u8], start: usize) -> Option<usize> {
        match bytes[start] {
            0x1b => escape_end(bytes, start + 1),
            0xc2..=0xdf => codepoint_end(bytes, start, 2),
            0xe0..=0xef => codepoint_end(bytes, start, 3),
            0xf0..=0xf4 => codepoint_end(bytes, start, 4),
            _ => Some(start + 1),
        }
    }

    fn codepoint_end(bytes: &[u8], start: usize, width: usize) -> Option<usize> {
        let end = start + width;
        (end <= bytes.len()).then_some(end)
    }

    fn escape_end(bytes: &[u8], mut i: usize) -> Option<usize> {
        loop {
            let byte = *bytes.get(i)?;
            i += 1;
            match byte {
                b'[' => return csi_end(bytes, i),
                b']' | b'P' | b'X' | b'^' | b'_' => return string_end(bytes, i),
                0x20..=0x2f => return intermediate_end(bytes, i),
                0x30..=0x7e => return Some(i),
                _ => {}
            }
        }
    }

    fn csi_end(bytes: &[u8], mut i: usize) -> Option<usize> {
        loop {
            let byte = *bytes.get(i)?;
            i += 1;
            match byte {
                0x40..=0x7e => return Some(i),
                0x1b => return escape_end(bytes, i),
                _ => {}
            }
        }
    }

    fn intermediate_end(bytes: &[u8], mut i: usize) -> Option<usize> {
        loop {
            let byte = *bytes.get(i)?;
            i += 1;
            match byte {
                0x30..=0x7e => return Some(i),
                0x1b => return escape_end(bytes, i),
                _ => {}
            }
        }
    }

    fn string_end(bytes: &[u8], mut i: usize) -> Option<usize> {
        loop {
            let byte = *bytes.get(i)?;
            i += 1;
            match byte {
                0x07 => return Some(i),
                0x1b => return escape_end(bytes, i),
                _ => {}
            }
        }
    }

    #[test]
    fn every_split_point_forwards_the_whole_input_without_bisecting_sequences() {
        let mut inputs = corpus();
        inputs.push(mixed_stream());
        for input in inputs {
            for cut in 0..=input.len() {
                let mut hold_back = HoldBack::new();
                let first = hold_back.feed(&input[..cut]);
                let second = hold_back.feed(&input[cut..]);
                let tail = hold_back.flush();
                assert!(
                    ends_on_complete_sequences(&first),
                    "first part bisects a sequence at cut {cut}"
                );
                assert!(
                    ends_on_complete_sequences(&second),
                    "second part bisects a sequence at cut {cut}"
                );
                assert!(
                    tail.is_empty(),
                    "well-formed input left a tail at cut {cut}"
                );
                let mut forwarded = first;
                forwarded.extend_from_slice(&second);
                assert_eq!(forwarded, input, "bytes lost or reordered at cut {cut}");
            }
        }
    }

    #[test]
    fn chunked_feeding_of_any_size_matches_the_unsplit_stream() {
        let input = mixed_stream();
        for chunk_size in 1..=17 {
            let mut hold_back = HoldBack::new();
            let mut forwarded = Vec::new();
            for chunk in input.chunks(chunk_size) {
                let out = hold_back.feed(chunk);
                assert!(
                    ends_on_complete_sequences(&out),
                    "chunk size {chunk_size} produced an unsafe boundary"
                );
                forwarded.extend(out);
            }
            forwarded.extend(hold_back.flush());
            assert_eq!(forwarded, input, "chunk size {chunk_size} lost bytes");
        }
    }

    #[test]
    fn a_firehose_split_at_read_buffer_boundaries_is_forwarded_byte_for_byte() {
        let input = firehose(20_000);
        for read_size in [1024, 4096, 8192, 8191, 65_536] {
            let mut hold_back = HoldBack::new();
            let mut forwarded = Vec::with_capacity(input.len());
            for chunk in input.chunks(read_size) {
                let out = hold_back.feed(chunk);
                assert!(
                    ends_on_complete_sequences(&out),
                    "read size {read_size} produced an unsafe boundary"
                );
                forwarded.extend(out);
            }
            forwarded.extend(hold_back.flush());
            assert_eq!(forwarded, input, "read size {read_size} altered the stream");
        }
    }

    #[test]
    fn the_vt100_screen_is_identical_with_and_without_hold_back() {
        let input = firehose(2_000);
        for read_size in [512, 8192, 8191] {
            let mut direct = vt100::Parser::new(24, 80, 1_000);
            direct.process(&input);

            let mut through = vt100::Parser::new(24, 80, 1_000);
            let mut hold_back = HoldBack::new();
            for chunk in input.chunks(read_size) {
                through.process(&hold_back.feed(chunk));
            }
            through.process(&hold_back.flush());

            assert_eq!(
                through.screen().contents_formatted(),
                direct.screen().contents_formatted(),
                "snapshot diverged at read size {read_size}"
            );
        }
    }

    #[test]
    fn an_unterminated_sequence_beyond_the_cap_flushes_degraded_and_resets_state() {
        let mut input = b"\x1b]52;c;".to_vec();
        input.resize(input.len() + HOLD_BACK_CAP + 1024, b'A');
        let mut hold_back = HoldBack::new();
        let mut forwarded = Vec::new();
        for chunk in input.chunks(8192) {
            forwarded.extend(hold_back.feed(chunk));
        }
        assert_eq!(forwarded, input);
        assert!(hold_back.flush().is_empty());
        assert_eq!(hold_back.feed(b"\x1b[31m"), b"\x1b[31m".to_vec());
    }

    #[test]
    fn esc_ending_an_osc_payload_is_held_as_a_potential_string_terminator() {
        let mut hold_back = HoldBack::new();
        assert!(hold_back
            .feed(b"\x1b]8;;https://example.com\x1b")
            .is_empty());
        assert_eq!(
            hold_back.feed(b"\\link"),
            b"\x1b]8;;https://example.com\x1b\\link".to_vec()
        );
    }

    #[test]
    fn esc_ending_an_osc_payload_that_opens_a_new_sequence_keeps_the_run_held() {
        let mut hold_back = HoldBack::new();
        assert!(hold_back.feed(b"\x1b]0;title\x1b").is_empty());
        assert!(hold_back.feed(b"[3").is_empty());
        assert_eq!(hold_back.feed(b"1m"), b"\x1b]0;title\x1b[31m".to_vec());
    }

    #[test]
    fn a_lone_trailing_esc_is_held() {
        let mut hold_back = HoldBack::new();
        assert_eq!(hold_back.feed(b"text\x1b"), b"text".to_vec());
        assert_eq!(hold_back.feed(b"[0m"), b"\x1b[0m".to_vec());
    }

    #[test]
    fn a_csi_without_its_final_byte_is_held() {
        let mut hold_back = HoldBack::new();
        assert_eq!(hold_back.feed(b"ok\x1b[38;5;19"), b"ok".to_vec());
        assert_eq!(hold_back.feed(b"6m"), b"\x1b[38;5;196m".to_vec());
    }

    #[test]
    fn a_dcs_without_st_is_held() {
        let mut hold_back = HoldBack::new();
        assert!(hold_back.feed(b"\x1bP1$r0m").is_empty());
        assert_eq!(
            hold_back.feed(b"\x1b\\done"),
            b"\x1bP1$r0m\x1b\\done".to_vec()
        );
    }

    #[test]
    fn a_split_utf8_codepoint_is_held() {
        let mut hold_back = HoldBack::new();
        assert_eq!(hold_back.feed(b"a\xf0\x9f"), b"a".to_vec());
        assert_eq!(hold_back.feed(b"\x9a\x80b"), b"\xf0\x9f\x9a\x80b".to_vec());
    }

    #[test]
    fn flush_returns_the_held_tail_and_resets_state() {
        let mut hold_back = HoldBack::new();
        assert!(hold_back.feed(b"\x1b[3").is_empty());
        assert_eq!(hold_back.flush(), b"\x1b[3".to_vec());
        assert_eq!(hold_back.feed(b"x"), b"x".to_vec());
    }

    #[test]
    fn a_malformed_utf8_lead_interrupted_by_other_bytes_passes_through() {
        let mut hold_back = HoldBack::new();
        assert_eq!(hold_back.feed(b"\xf0hi"), b"\xf0hi".to_vec());
        assert_eq!(hold_back.feed(b"\xe0\x80\x1b"), b"\xe0\x80".to_vec());
        assert_eq!(hold_back.feed(b"[m"), b"\x1b[m".to_vec());
    }
}
