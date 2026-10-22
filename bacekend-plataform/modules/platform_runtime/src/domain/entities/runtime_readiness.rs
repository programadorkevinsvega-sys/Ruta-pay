#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeReadiness {
    Ready,
}

impl RuntimeReadiness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
        }
    }
}
