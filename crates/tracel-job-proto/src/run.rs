//! Running a job: the number a launcher gives it, and how the binary's exit code says it ended.

/// Gives the number a launcher gave the job it runs, which the job reads as its number.
pub const TRACEL_JOB_NUM: &str = "TRACEL_JOB_NUM";

/// The exit code of a job binary, which says how its job ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    /// 0: the job completed, or the binary printed its help, its version or a completion script,
    /// or wrote its definitions file.
    Completed,
    /// 1: the job failed, or the definitions file could not be written.
    Failed,
    /// 2: the binary rejected its command line: no job or an unknown job is named, and stderr
    /// lists the job names; or a flag or the input is unusable or does not decode.
    Rejected,
}

impl ExitCode {
    /// The code the binary exits with.
    pub const fn code(self) -> u8 {
        match self {
            Self::Completed => 0,
            Self::Failed => 1,
            Self::Rejected => 2,
        }
    }

    /// The exit code a binary that exited with `code` gives, when it is one of the protocol's.
    pub fn from_code(code: i32) -> Option<Self> {
        [Self::Completed, Self::Failed, Self::Rejected]
            .into_iter()
            .find(|exit| i32::from(exit.code()) == code)
    }
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(exit: ExitCode) -> Self {
        Self::from(exit.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_exit_code_reads_back_from_its_code() {
        for (exit, code) in [
            (ExitCode::Completed, 0),
            (ExitCode::Failed, 1),
            (ExitCode::Rejected, 2),
        ] {
            assert_eq!(exit.code(), code);
            assert_eq!(ExitCode::from_code(i32::from(code)), Some(exit));
        }
        for code in [-1, 3, 101, 130, 137, 256] {
            assert_eq!(ExitCode::from_code(code), None);
        }
    }
}
