pub mod instruments;
pub mod patterns;
pub mod scanner;
pub mod signal;
pub mod types;
pub mod grpc;

pub mod proto {
    tonic::include_proto!("rthmn");
}
