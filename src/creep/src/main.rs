mod config;
mod index;
mod proto {
    tonic::include_proto!("creep.v1");
}
mod service;
mod watcher;

fn main() {
    println!("creep placeholder");
}
