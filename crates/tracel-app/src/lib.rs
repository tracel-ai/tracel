/// Command-line interface for dispatching registered jobs.
pub mod cli;
/// Configuration mappers for deserializing job inputs. (Mostly use for the cli)
pub mod mapper;

mod job;
mod job_register;
