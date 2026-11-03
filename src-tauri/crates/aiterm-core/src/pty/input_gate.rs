//! 使用者輸入的閘門：後端正在替一個分頁「打字」（例如派工把提示詞貼進 Claude）時，
//! 使用者同時從前端打的字會跟它交錯，變成同一則訊息。閘門關著時先把使用者的輸入暫存起來，
//! 後端打完（放行）之後再依原順序送進去，使用者打的字不會遺失。

#[derive(Debug, Default)]
pub struct InputGate {
    locked: bool,
    pending: Vec<u8>,
}

impl InputGate {
    /// 關閘：之後使用者的輸入先暫存。
    pub fn lock(&mut self) {
        self.locked = true;
    }

    /// 使用者的輸入：閘門開著就回傳現在要寫的位元組，關著就暫存、回傳 `None`。
    pub fn submit(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        if self.locked {
            self.pending.extend_from_slice(data);
            None
        } else {
            Some(data.to_vec())
        }
    }

    /// 放行：開閘，回傳暫存的輸入（依原順序）。
    pub fn release(&mut self) -> Vec<u8> {
        self.locked = false;
        std::mem::take(&mut self.pending)
    }

    pub fn is_locked(&self) -> bool {
        self.locked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_gate_passes_input_straight_through() {
        let mut g = InputGate::default();
        assert_eq!(g.submit(b"abc"), Some(b"abc".to_vec()));
        assert!(!g.is_locked());
    }

    #[test]
    fn a_locked_gate_holds_input_and_releases_it_in_order() {
        let mut g = InputGate::default();
        g.lock();
        assert_eq!(g.submit("請幫我".as_bytes()), None);
        assert_eq!(g.submit("看一下\r".as_bytes()), None);
        assert!(g.is_locked());
        assert_eq!(g.release(), "請幫我看一下\r".as_bytes().to_vec());
        assert!(!g.is_locked());
        // 放行之後恢復直通，暫存也清空了。
        assert_eq!(g.submit(b"x"), Some(b"x".to_vec()));
        assert_eq!(g.release(), Vec::<u8>::new());
    }

    #[test]
    fn releasing_an_unlocked_gate_is_harmless() {
        let mut g = InputGate::default();
        assert_eq!(g.release(), Vec::<u8>::new());
    }
}
