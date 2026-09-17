use std::io::{self, Read, Write};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// 雙向的原始位元組：主行程→sidecar 是鍵盤輸入，sidecar→主行程是 ConPTY 輸出。
    Data(Vec<u8>),
    /// 主行程通知 sidecar 調整 ConPTY 大小。
    Resize { cols: u16, rows: u16 },
    /// 任一端準備關閉連線前的最後一個 frame。
    Exit,
}

const KIND_DATA: u8 = 0;
const KIND_RESIZE: u8 = 1;
const KIND_EXIT: u8 = 2;

impl Frame {
    /// 寫入格式：4 bytes little-endian payload 長度 + 1 byte 種類 + payload。
    /// 長度**只算 payload**，不含種類位元組本身。
    pub fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        match self {
            Frame::Data(bytes) => {
                w.write_all(&(bytes.len() as u32).to_le_bytes())?;
                w.write_all(&[KIND_DATA])?;
                w.write_all(bytes)?;
            }
            Frame::Resize { cols, rows } => {
                w.write_all(&4u32.to_le_bytes())?;
                w.write_all(&[KIND_RESIZE])?;
                w.write_all(&cols.to_le_bytes())?;
                w.write_all(&rows.to_le_bytes())?;
            }
            Frame::Exit => {
                w.write_all(&0u32.to_le_bytes())?;
                w.write_all(&[KIND_EXIT])?;
            }
        }
        Ok(())
    }

    /// 讀一個完整 frame。EOF 在長度前綴之前發生時回傳 `Ok(None)`；讀到一半才
    /// EOF 一律當錯誤，不能把不完整的 frame 當成合法資料處理。
    pub fn read_from<R: Read>(r: &mut R) -> io::Result<Option<Frame>> {
        let mut len_buf = [0u8; 4];
        match r.read(&mut len_buf[..1])? {
            0 => return Ok(None),
            _ => r.read_exact(&mut len_buf[1..])?,
        }
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut kind_buf = [0u8; 1];
        r.read_exact(&mut kind_buf)?;

        match kind_buf[0] {
            KIND_DATA => {
                let mut payload = vec![0u8; len];
                r.read_exact(&mut payload)?;
                Ok(Some(Frame::Data(payload)))
            }
            KIND_RESIZE => {
                if len != 4 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "resize frame must be 4 bytes"));
                }
                let mut buf = [0u8; 4];
                r.read_exact(&mut buf)?;
                let cols = u16::from_le_bytes([buf[0], buf[1]]);
                let rows = u16::from_le_bytes([buf[2], buf[3]]);
                Ok(Some(Frame::Resize { cols, rows }))
            }
            KIND_EXIT => Ok(Some(Frame::Exit)),
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown frame kind: {other}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn data_frame_round_trips() {
        let frame = Frame::Data(b"hello".to_vec());
        let mut buf = Vec::new();
        frame.write_to(&mut buf).unwrap();
        let mut cursor = Cursor::new(buf);
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), Some(frame));
    }

    #[test]
    fn resize_frame_round_trips() {
        let frame = Frame::Resize { cols: 120, rows: 40 };
        let mut buf = Vec::new();
        frame.write_to(&mut buf).unwrap();
        let mut cursor = Cursor::new(buf);
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), Some(frame));
    }

    #[test]
    fn exit_frame_round_trips() {
        let frame = Frame::Exit;
        let mut buf = Vec::new();
        frame.write_to(&mut buf).unwrap();
        let mut cursor = Cursor::new(buf);
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), Some(frame));
    }

    #[test]
    fn empty_stream_yields_none() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), None);
    }

    #[test]
    fn two_frames_back_to_back_both_read() {
        let mut buf = Vec::new();
        Frame::Data(b"a".to_vec()).write_to(&mut buf).unwrap();
        Frame::Data(b"bb".to_vec()).write_to(&mut buf).unwrap();
        let mut cursor = Cursor::new(buf);
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), Some(Frame::Data(b"a".to_vec())));
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), Some(Frame::Data(b"bb".to_vec())));
        assert_eq!(Frame::read_from(&mut cursor).unwrap(), None);
    }

    #[test]
    fn truncated_frame_is_an_error_not_none() {
        let mut buf = Vec::new();
        Frame::Data(b"hello".to_vec()).write_to(&mut buf).unwrap();
        buf.truncate(3); // cut mid-length-prefix
        let mut cursor = Cursor::new(buf);
        assert!(Frame::read_from(&mut cursor).is_err());
    }
}
