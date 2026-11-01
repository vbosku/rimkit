//! Mod preview images (About/Preview.png), decoded and downscaled on worker threads.
//! `get` never blocks: it returns None until the texture is ready, then the UI repaints.

use eframe::egui::{ColorImage, Context, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, mpsc};

const WORKERS: usize = 3;
/// Requests beyond this are stale (scrolled away); the oldest get dropped and re-requested if seen again.
const MAX_QUEUE: usize = 48;

type Key = (PathBuf, u32);

enum Slot {
    Loading,
    Ready(TextureHandle),
    Missing,
}

#[derive(Default)]
struct Queue {
    jobs: Mutex<Vec<Key>>,
    ready: Condvar,
}

pub struct Previews {
    queue: Arc<Queue>,
    done: mpsc::Receiver<(Key, Option<ColorImage>)>,
    slots: HashMap<Key, Slot>,
}

impl Previews {
    pub fn new(ctx: &Context) -> Self {
        let queue = Arc::new(Queue::default());
        let (tx, done) = mpsc::channel();
        for _ in 0..WORKERS {
            let (queue, tx, ctx) = (queue.clone(), tx.clone(), ctx.clone());
            std::thread::spawn(move || loop {
                let key = {
                    let mut jobs = queue.jobs.lock().unwrap();
                    loop {
                        // Newest first: what's on screen right now.
                        if let Some(k) = jobs.pop() {
                            break k;
                        }
                        jobs = queue.ready.wait(jobs).unwrap();
                    }
                };
                let image = decode(&key.0, key.1);
                if tx.send((key, image)).is_err() {
                    return;
                }
                ctx.request_repaint();
            });
        }
        Self { queue, done, slots: HashMap::new() }
    }

    /// The preview of the mod in `mod_dir`, fitted in `size`×`size` pixels. None while loading or if it has none.
    pub fn get(&mut self, mod_dir: &Path, size: u32) -> Option<&TextureHandle> {
        let key = (mod_dir.join("About").join("Preview.png"), size);
        if !self.slots.contains_key(&key) {
            self.slots.insert(key.clone(), Slot::Loading);
            let mut jobs = self.queue.jobs.lock().unwrap();
            jobs.push(key.clone());
            if jobs.len() > MAX_QUEUE {
                let excess = jobs.len() - MAX_QUEUE;
                let stale: Vec<Key> = jobs.drain(..excess).collect();
                for k in stale {
                    self.slots.remove(&k);
                }
            }
            self.queue.ready.notify_one();
        }
        match self.slots.get(&key) {
            Some(Slot::Ready(t)) => Some(t),
            _ => None,
        }
    }

    /// Uploads finished decodes to the GPU. Call once per frame.
    pub fn poll(&mut self, ctx: &Context) {
        for (key, image) in self.done.try_iter() {
            let slot = match image {
                Some(img) => Slot::Ready(ctx.load_texture(key.0.to_string_lossy(), img, TextureOptions::LINEAR)),
                None => Slot::Missing,
            };
            self.slots.insert(key, slot);
        }
    }

    pub fn pending(&self) -> usize {
        self.slots.values().filter(|s| matches!(s, Slot::Loading)).count()
    }

    /// Forget everything (e.g. after a rescan, images may have changed).
    pub fn clear(&mut self) {
        self.queue.jobs.lock().unwrap().clear();
        self.slots.clear();
    }
}

fn decode(path: &Path, size: u32) -> Option<ColorImage> {
    let bytes = std::fs::read(path).ok()?;
    let rgba = image::load_from_memory(&bytes).ok()?.thumbnail(size, size).to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], &rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Times decoding every workshop preview; run with `cargo test --release decode_all -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn decode_all() {
        let root = Path::new(r"C:\Program Files (x86)\Steam\steamapps\workshop\content\294100");
        let dirs: Vec<PathBuf> = std::fs::read_dir(root).unwrap().flatten().map(|e| e.path()).collect();
        let t = std::time::Instant::now();
        let n = dirs.iter().filter(|d| decode(&d.join("About").join("Preview.png"), 128).is_some()).count();
        println!("decoded {n} previews at 128px in {:?} ({:?} each)", t.elapsed(), t.elapsed() / n.max(1) as u32);
    }
}
