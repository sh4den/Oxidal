pub enum BackendEvent {
    Data(Vec<u8>),
    Closed(Option<String>),
}

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
