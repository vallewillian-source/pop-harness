use std::io;

use tokio::io::{AsyncRead, AsyncReadExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextLine {
    Line(String),
    Discarded,
    End,
}

pub struct LineReader<R> {
    input: R,
    buffer: Vec<u8>,
    chunk: Box<[u8; 8192]>,
    limit: usize,
    discarding: bool,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    pub fn new(input: R, limit: usize) -> Self {
        Self {
            input,
            buffer: Vec::new(),
            chunk: Box::new([0u8; 8192]),
            limit: limit.max(1),
            discarding: false,
        }
    }

    pub async fn next_line(&mut self) -> io::Result<NextLine> {
        loop {
            if self.discarding {
                if let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
                    self.buffer.drain(..=position);
                    self.discarding = false;
                    return Ok(NextLine::Discarded);
                }
                self.buffer.clear();
            } else if let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
                if position > self.limit {
                    self.buffer.drain(..=position);
                    return Ok(NextLine::Discarded);
                }
                let mut line: Vec<u8> = self.buffer.drain(..=position).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(NextLine::Line(String::from_utf8_lossy(&line).into_owned()));
            } else if self.buffer.len() > self.limit {
                self.buffer.clear();
                self.discarding = true;
                continue;
            }

            let read = self.input.read(&mut self.chunk[..]).await?;
            if read == 0 {
                if self.discarding {
                    self.discarding = false;
                    return Ok(NextLine::Discarded);
                }
                return Ok(NextLine::End);
            }
            self.buffer.extend_from_slice(&self.chunk[..read]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_lines_with_crlf_and_empty_line() {
        let data = b"{\"a\":1}\r\n{\"b\":2}\n\n";
        let mut reader = LineReader::new(&data[..], 1024);
        assert_eq!(
            reader.next_line().await.unwrap(),
            NextLine::Line("{\"a\":1}".into())
        );
        assert_eq!(
            reader.next_line().await.unwrap(),
            NextLine::Line("{\"b\":2}".into())
        );
        assert_eq!(
            reader.next_line().await.unwrap(),
            NextLine::Line(String::new())
        );
        assert_eq!(reader.next_line().await.unwrap(), NextLine::End);
    }

    #[tokio::test]
    async fn giant_line_is_discarded_and_the_next_one_returns() {
        let mut data = Vec::new();
        data.extend(std::iter::repeat_n(b'x', 4096));
        data.push(b'\n');
        data.extend_from_slice(b"{\"ok\":true}\n");
        let mut reader = LineReader::new(&data[..], 256);
        assert_eq!(reader.next_line().await.unwrap(), NextLine::Discarded);
        assert_eq!(
            reader.next_line().await.unwrap(),
            NextLine::Line("{\"ok\":true}".into())
        );
        assert_eq!(reader.next_line().await.unwrap(), NextLine::End);
    }

    #[tokio::test]
    async fn unterminated_line_above_the_cap_is_discarded_at_the_end() {
        let data: Vec<u8> = std::iter::repeat_n(b'y', 1000).collect();
        let mut reader = LineReader::new(&data[..], 128);
        assert_eq!(reader.next_line().await.unwrap(), NextLine::Discarded);
        assert_eq!(reader.next_line().await.unwrap(), NextLine::End);
    }
}
