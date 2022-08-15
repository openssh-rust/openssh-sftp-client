use std::{mem, sync::Mutex};

use bytes::Bytes;
use openssh_sftp_client_lowlevel::Queue;

#[derive(Default)]
pub struct MpscQueue(Mutex<Vec<Bytes>>);

impl MpscQueue {
    pub fn consume(&self) -> Vec<Bytes> {
        mem::take(&mut *self.0.lock().unwrap())
    }
}

impl Queue for MpscQueue {
    fn push(&self, bytes: Bytes) {
        self.0.lock().unwrap().push(bytes);
    }

    fn extend(&self, header: Bytes, body: &[&[Bytes]]) {
        let mut v = self.0.lock().unwrap();
        v.push(header);
        for data in body {
            v.extend(data.iter().cloned());
        }
    }
}
