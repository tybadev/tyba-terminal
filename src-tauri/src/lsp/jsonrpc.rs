use std::io::{self, BufRead, Write};

pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

pub fn write_message<W: Write>(writer: &mut W, body: &[u8]) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)?;
    writer.flush()
}

fn parse_content_length(header: &str) -> Option<usize> {
    let (name, value) = header.split_once(':')?;
    if !name.trim().eq_ignore_ascii_case("content-length") {
        return None;
    }
    value.trim().parse::<usize>().ok()
}

pub fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut content_length: Option<usize> = None;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(None);
        }
        let trimmed = trim_crlf(&line);
        if trimmed.is_empty() {
            break;
        }
        if let Ok(text) = std::str::from_utf8(trimmed) {
            if let Some(len) = parse_content_length(text) {
                content_length = Some(len);
            }
        }
    }
    let len = match content_length {
        Some(len) => len,
        None => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame LSP sem Content-Length",
            ))
        }
    };
    if len > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame LSP excede o teto de tamanho",
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

fn trim_crlf(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    while end > 0 && (line[end - 1] == b'\n' || line[end - 1] == b'\r') {
        end -= 1;
    }
    &line[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Read};

    struct DripReader {
        data: Vec<u8>,
        pos: usize,
        chunk: usize,
    }

    impl DripReader {
        fn new(data: Vec<u8>, chunk: usize) -> Self {
            Self {
                data,
                pos: 0,
                chunk,
            }
        }
    }

    impl Read for DripReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let take = self.chunk.min(buf.len()).min(self.data.len() - self.pos);
            buf[..take].copy_from_slice(&self.data[self.pos..self.pos + take]);
            self.pos += take;
            Ok(take)
        }
    }

    fn frame(body: &str) -> Vec<u8> {
        let mut out = Vec::new();
        write_message(&mut out, body.as_bytes()).unwrap();
        out
    }

    #[test]
    fn round_trip_single_message() {
        let bytes = frame(r#"{"jsonrpc":"2.0","id":1}"#);
        let mut reader = BufReader::new(&bytes[..]);
        let body = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(body, br#"{"jsonrpc":"2.0","id":1}"#);
    }

    #[test]
    fn reads_multiple_concatenated_messages() {
        let mut stream = frame(r#"{"a":1}"#);
        stream.extend(frame(r#"{"b":2}"#));
        stream.extend(frame(r#"{"c":3}"#));
        let mut reader = BufReader::new(&stream[..]);
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), br#"{"a":1}"#);
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), br#"{"b":2}"#);
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), br#"{"c":3}"#);
        assert!(read_message(&mut reader).unwrap().is_none());
    }

    #[test]
    fn survives_a_reader_that_yields_one_byte_at_a_time() {
        let mut stream = frame(r#"{"first":true}"#);
        stream.extend(frame(r#"{"second":true}"#));
        let mut reader = BufReader::new(DripReader::new(stream, 1));
        assert_eq!(
            read_message(&mut reader).unwrap().unwrap(),
            br#"{"first":true}"#
        );
        assert_eq!(
            read_message(&mut reader).unwrap().unwrap(),
            br#"{"second":true}"#
        );
    }

    #[test]
    fn content_length_counts_bytes_not_chars_across_utf8_boundary() {
        let payload = r#"{"msg":"café — αβγ 日本語"}"#;
        let expected = payload.as_bytes().to_vec();
        let bytes = frame(payload);
        assert!(bytes.starts_with(format!("Content-Length: {}\r\n\r\n", expected.len()).as_bytes()));
        let mut reader = BufReader::new(DripReader::new(bytes, 1));
        let body = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(body, expected);
        assert_eq!(std::str::from_utf8(&body).unwrap(), payload);
    }

    #[test]
    fn header_name_is_case_insensitive_and_ignores_extra_headers() {
        let mut stream = Vec::new();
        let body = br#"{"ok":1}"#;
        write!(
            &mut stream,
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncOnTeNt-LeNgTh: {}\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.extend_from_slice(body);
        let mut reader = BufReader::new(&stream[..]);
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), body);
    }

    #[test]
    fn clean_eof_before_any_header_is_none_not_error() {
        let empty: &[u8] = b"";
        let mut reader = BufReader::new(empty);
        assert!(read_message(&mut reader).unwrap().is_none());
    }

    #[test]
    fn missing_content_length_is_an_error() {
        let stream = b"X-Header: 1\r\n\r\n".to_vec();
        let mut reader = BufReader::new(&stream[..]);
        assert!(read_message(&mut reader).is_err());
    }
}
