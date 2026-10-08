//! The number a launcher gives the job it runs.

/// Gives the number a launcher gave the job it runs, which each experiment the job starts records
/// as its [`JOB_NUM_ATTRIBUTE`].
pub const TRACEL_JOB_NUM: &str = "TRACEL_JOB_NUM";

/// The experiment attribute that links an experiment to the job that started it: the job's
/// [`TRACEL_JOB_NUM`], as a number when it is one.
pub const JOB_NUM_ATTRIBUTE: &str = "tracel.job_num";
