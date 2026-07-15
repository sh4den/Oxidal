/// A message produced by a terminal backend (local shell, SSH, serial, ...).
pub enum BackendEvent {
    Data(Vec<u8>),
    /// The backend connection ended. `Some(message)` if it ended due to an error.
    Closed(Option<String>),
}

/// A running terminal backend: something that produces a byte stream and
/// accepts input bytes + resize requests. Local PTYs, SSH shells and serial
/// ports all implement this same shape so `TerminalView` can host any of them.
pub struct Backend {
    pub events: async_channel::Receiver<BackendEvent>,
    input: async_channel::Sender<Vec<u8>>,
    resize: async_channel::Sender<(u16, u16)>,
}

impl Backend {
    pub fn new(
        events: async_channel::Receiver<BackendEvent>,
        input: async_channel::Sender<Vec<u8>>,
        resize: async_channel::Sender<(u16, u16)>,
    ) -> Self {
        Self {
            events,
            input,
            resize,
        }
    }

    pub fn write_input(&self, data: &[u8]) {
        let _ = self.input.send_blocking(data.to_vec());
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let _ = self.resize.send_blocking((rows, cols));
    }
}
