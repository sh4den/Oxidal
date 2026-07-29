pub enum BackendEvent {
    Data(Vec<u8>),
    Closed(Option<String>),
}

pub struct Backend {
    pub events: async_channel::Receiver<BackendEvent>,
    input: async_channel::Sender<Vec<u8>>,
    // None for transports with no resize concept (e.g. serial).
    resize: Option<async_channel::Sender<(u16, u16)>>,
    // Transports whose threads block on the OS (serial reads, PTY reads) can't
    // notice a closed channel on their own, so they hand us a way to wake them.
    shutdown: Option<Box<dyn FnOnce() + Send>>,
}

impl Backend {
    pub fn new(
        events: async_channel::Receiver<BackendEvent>,
        input: async_channel::Sender<Vec<u8>>,
        resize: Option<async_channel::Sender<(u16, u16)>>,
    ) -> Self {
        Self {
            events,
            input,
            resize,
            shutdown: None,
        }
    }

    pub fn on_shutdown(mut self, shutdown: impl FnOnce() + Send + 'static) -> Self {
        self.shutdown = Some(Box::new(shutdown));
        self
    }

    pub fn write_input(&self, data: &[u8]) {
        let _ = self.input.send_blocking(data.to_vec());
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        if let Some(resize) = &self.resize {
            let _ = resize.send_blocking((rows, cols));
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        // The view's reader task holds its own clone of `events`, so closing the
        // channels here is what actually tells the transport threads to stop.
        self.input.close();
        self.events.close();
        if let Some(resize) = &self.resize {
            resize.close();
        }
        if let Some(shutdown) = self.shutdown.take() {
            shutdown();
        }
    }
}
