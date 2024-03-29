use std::{mem, sync::Mutex};

use bytes::Bytes;
use openssh_sftp_client_lowlevel::Queue;

#[derive(Debug)]
pub(super) struct MpscQueue(Mutex<Vec<Bytes>>);

impl MpscQueue {
    pub(crate) fn with_capacity(cap: usize) -> Self {
        Self(Mutex::new(Vec::with_capacity(cap)))
    }

    pub(crate) fn swap(&self, backup_vec: &mut Vec<Bytes>) {
        mem::swap(&mut *self.0.lock().unwrap(), backup_vec)
    }
}

impl Queue for MpscQueue {
    fn push(&self, bytes: Bytes) {
        if !bytes.is_empty() {
            self.0.lock().unwrap().push(bytes);
        }
    }

    fn extend(&self, header: Bytes, body: &[&[Bytes]]) {
        let mut v = self.0.lock().unwrap();

        if !header.is_empty() {
            v.push(header);
        }

        for data in body {
            v.extend(data.iter().filter(|bytes| !bytes.is_empty()).cloned());
        }
    }
}
