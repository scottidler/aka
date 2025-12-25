//! System abstraction layer for mocking external resources
//!
//! This module provides traits for system operations that can be mocked in tests:
//! - Socket connections (Unix sockets)
//! - Process execution (Command)
//! - File system operations

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Output;
use std::time::Duration;

// ============================================================================
// Socket Abstractions
// ============================================================================

/// Trait for socket connections - allows mocking Unix socket I/O
pub trait SocketStream: Read + Write + Send + std::fmt::Debug {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
}

/// Trait for creating socket connections
pub trait SocketConnector: Send + Sync {
    fn connect(&self, path: &Path) -> io::Result<Box<dyn SocketStream>>;
    fn path_exists(&self, path: &Path) -> bool;
    fn is_socket(&self, path: &Path) -> io::Result<bool>;
}

/// Real Unix socket implementation
#[derive(Default)]
pub struct RealSocketConnector;

impl SocketConnector for RealSocketConnector {
    fn connect(&self, path: &Path) -> io::Result<Box<dyn SocketStream>> {
        let stream = std::os::unix::net::UnixStream::connect(path)?;
        Ok(Box::new(RealSocketStream(stream)))
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_socket(&self, path: &Path) -> io::Result<bool> {
        use std::os::unix::fs::FileTypeExt;
        let metadata = std::fs::metadata(path)?;
        Ok(metadata.file_type().is_socket())
    }
}

#[derive(Debug)]
struct RealSocketStream(std::os::unix::net::UnixStream);

impl Read for RealSocketStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for RealSocketStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl SocketStream for RealSocketStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_read_timeout(timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_write_timeout(timeout)
    }
}

// ============================================================================
// Process/Command Abstractions
// ============================================================================

/// Result of running a command
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub success: bool,
    pub code: Option<i32>,
}

impl CommandOutput {
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }
}

impl From<Output> for CommandOutput {
    fn from(output: Output) -> Self {
        Self {
            stdout: output.stdout,
            stderr: output.stderr,
            success: output.status.success(),
            code: output.status.code(),
        }
    }
}

/// Trait for running system commands
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput>;
}

/// Real command runner using std::process::Command
#[derive(Default)]
pub struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<CommandOutput> {
        let output = std::process::Command::new(program).args(args).output()?;
        Ok(output.into())
    }
}

// ============================================================================
// File System Abstractions
// ============================================================================

/// Trait for file system operations
pub trait FileSystem: Send + Sync {
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn is_file(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
}

/// Real file system implementation
#[derive(Default)]
pub struct RealFileSystem;

impl FileSystem for RealFileSystem {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        std::fs::write(path, contents)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
}

// ============================================================================
// Mock Implementations (for testing)
// ============================================================================

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    // ------------------------------------------------------------------------
    // Mock Socket
    // ------------------------------------------------------------------------

    /// Mock socket stream for testing
    #[derive(Debug)]
    pub struct MockSocketStream {
        read_data: Cursor<Vec<u8>>,
        write_data: Arc<Mutex<Vec<u8>>>,
        fail_read: bool,
        fail_write: bool,
        read_error_kind: io::ErrorKind,
        write_error_kind: io::ErrorKind,
    }

    impl MockSocketStream {
        pub fn new(response: &str) -> Self {
            Self {
                read_data: Cursor::new(response.as_bytes().to_vec()),
                write_data: Arc::new(Mutex::new(Vec::new())),
                fail_read: false,
                fail_write: false,
                read_error_kind: io::ErrorKind::Other,
                write_error_kind: io::ErrorKind::Other,
            }
        }

        pub fn with_read_error(mut self, kind: io::ErrorKind) -> Self {
            self.fail_read = true;
            self.read_error_kind = kind;
            self
        }

        pub fn with_write_error(mut self, kind: io::ErrorKind) -> Self {
            self.fail_write = true;
            self.write_error_kind = kind;
            self
        }

        pub fn written_data(&self) -> Vec<u8> {
            self.write_data.lock().unwrap().clone()
        }

        pub fn written_string(&self) -> String {
            String::from_utf8_lossy(&self.written_data()).to_string()
        }
    }

    impl Read for MockSocketStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.fail_read {
                return Err(io::Error::new(self.read_error_kind, "mock read error"));
            }
            self.read_data.read(buf)
        }
    }

    impl Write for MockSocketStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.fail_write {
                return Err(io::Error::new(self.write_error_kind, "mock write error"));
            }
            self.write_data.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SocketStream for MockSocketStream {
        fn set_read_timeout(&self, _: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
        fn set_write_timeout(&self, _: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }

    /// Mock socket connector for testing
    pub struct MockSocketConnector {
        response: Option<String>,
        connect_error: Option<io::ErrorKind>,
        socket_exists: bool,
        is_socket: bool,
    }

    impl MockSocketConnector {
        pub fn new(response: &str) -> Self {
            Self {
                response: Some(response.to_string()),
                connect_error: None,
                socket_exists: true,
                is_socket: true,
            }
        }

        pub fn connection_refused() -> Self {
            Self {
                response: None,
                connect_error: Some(io::ErrorKind::ConnectionRefused),
                socket_exists: true,
                is_socket: true,
            }
        }

        pub fn not_found() -> Self {
            Self {
                response: None,
                connect_error: Some(io::ErrorKind::NotFound),
                socket_exists: false,
                is_socket: false,
            }
        }

        pub fn timed_out() -> Self {
            Self {
                response: None,
                connect_error: Some(io::ErrorKind::TimedOut),
                socket_exists: true,
                is_socket: true,
            }
        }

        pub fn permission_denied() -> Self {
            Self {
                response: None,
                connect_error: Some(io::ErrorKind::PermissionDenied),
                socket_exists: true,
                is_socket: true,
            }
        }
    }

    impl SocketConnector for MockSocketConnector {
        fn connect(&self, _path: &Path) -> io::Result<Box<dyn SocketStream>> {
            if let Some(error_kind) = self.connect_error {
                return Err(io::Error::new(error_kind, "mock connection error"));
            }
            let response = self.response.as_ref().unwrap();
            Ok(Box::new(MockSocketStream::new(response)))
        }

        fn path_exists(&self, _path: &Path) -> bool {
            self.socket_exists
        }

        fn is_socket(&self, _path: &Path) -> io::Result<bool> {
            Ok(self.is_socket)
        }
    }

    // ------------------------------------------------------------------------
    // Mock Command Runner
    // ------------------------------------------------------------------------

    /// Mock command runner for testing
    #[derive(Default)]
    pub struct MockCommandRunner {
        responses: Arc<Mutex<HashMap<String, CommandOutput>>>,
        default_response: Option<CommandOutput>,
    }

    impl MockCommandRunner {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn expect(self, program: &str, output: CommandOutput) -> Self {
            self.responses.lock().unwrap().insert(program.to_string(), output);
            self
        }

        pub fn with_default(mut self, output: CommandOutput) -> Self {
            self.default_response = Some(output);
            self
        }

        /// Helper: expect pgrep to find a running process
        pub fn pgrep_running(self) -> Self {
            self.expect(
                "pgrep",
                CommandOutput {
                    stdout: b"12345\n".to_vec(),
                    stderr: Vec::new(),
                    success: true,
                    code: Some(0),
                },
            )
        }

        /// Helper: expect pgrep to find no process
        pub fn pgrep_not_running(self) -> Self {
            self.expect(
                "pgrep",
                CommandOutput {
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    success: false,
                    code: Some(1),
                },
            )
        }

        /// Helper: expect systemctl to succeed
        pub fn systemctl_success(self) -> Self {
            self.expect(
                "systemctl",
                CommandOutput {
                    stdout: b"active\n".to_vec(),
                    stderr: Vec::new(),
                    success: true,
                    code: Some(0),
                },
            )
        }

        /// Helper: expect systemctl to fail
        pub fn systemctl_failure(self) -> Self {
            self.expect(
                "systemctl",
                CommandOutput {
                    stdout: Vec::new(),
                    stderr: b"Failed to start\n".to_vec(),
                    success: false,
                    code: Some(1),
                },
            )
        }

        /// Helper: expect which to find a binary
        pub fn which_found(self, path: &str) -> Self {
            self.expect(
                "which",
                CommandOutput {
                    stdout: format!("{}\n", path).into_bytes(),
                    stderr: Vec::new(),
                    success: true,
                    code: Some(0),
                },
            )
        }

        /// Helper: expect which to not find a binary
        pub fn which_not_found(self) -> Self {
            self.expect(
                "which",
                CommandOutput {
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    success: false,
                    code: Some(1),
                },
            )
        }
    }

    impl CommandRunner for MockCommandRunner {
        fn run(&self, program: &str, _args: &[&str]) -> io::Result<CommandOutput> {
            let responses = self.responses.lock().unwrap();
            if let Some(output) = responses.get(program) {
                return Ok(output.clone());
            }
            if let Some(ref default) = self.default_response {
                return Ok(default.clone());
            }
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("command '{}' not mocked", program),
            ))
        }
    }

    // ------------------------------------------------------------------------
    // Mock File System
    // ------------------------------------------------------------------------

    /// Mock file system for testing
    #[derive(Default, Clone)]
    pub struct MockFileSystem {
        files: Arc<Mutex<HashMap<PathBuf, Vec<u8>>>>,
        directories: Arc<Mutex<Vec<PathBuf>>>,
    }

    impl MockFileSystem {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn with_file(self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Self {
            self.files
                .lock()
                .unwrap()
                .insert(path.as_ref().to_path_buf(), contents.as_ref().to_vec());
            self
        }

        pub fn with_dir(self, path: impl AsRef<Path>) -> Self {
            self.directories.lock().unwrap().push(path.as_ref().to_path_buf());
            self
        }

        pub fn get_file(&self, path: impl AsRef<Path>) -> Option<Vec<u8>> {
            self.files.lock().unwrap().get(path.as_ref()).cloned()
        }

        pub fn get_file_string(&self, path: impl AsRef<Path>) -> Option<String> {
            self.get_file(path).map(|b| String::from_utf8_lossy(&b).to_string())
        }
    }

    impl FileSystem for MockFileSystem {
        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            self.files
                .lock()
                .unwrap()
                .get(path)
                .map(|b| String::from_utf8_lossy(b).to_string())
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "file not found"))
        }

        fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
            self.files.lock().unwrap().insert(path.to_path_buf(), contents.to_vec());
            Ok(())
        }

        fn exists(&self, path: &Path) -> bool {
            self.files.lock().unwrap().contains_key(path)
                || self
                    .directories
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|d| d == path || path.starts_with(d))
        }

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            self.directories.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }

        fn is_file(&self, path: &Path) -> bool {
            self.files.lock().unwrap().contains_key(path)
        }

        fn is_dir(&self, path: &Path) -> bool {
            self.directories.lock().unwrap().contains(&path.to_path_buf())
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use mock::*;

    // Socket tests
    #[test]
    fn test_mock_socket_stream_read() {
        let mut stream = MockSocketStream::new("hello world");
        let mut buf = [0u8; 11];
        let n = stream.read(&mut buf).unwrap();
        assert_eq!(n, 11);
        assert_eq!(&buf, b"hello world");
    }

    #[test]
    fn test_mock_socket_stream_write() {
        let mut stream = MockSocketStream::new("");
        stream.write_all(b"test data").unwrap();
        assert_eq!(stream.written_string(), "test data");
    }

    #[test]
    fn test_mock_socket_stream_read_error() {
        let mut stream = MockSocketStream::new("").with_read_error(io::ErrorKind::TimedOut);
        let mut buf = [0u8; 10];
        let result = stream.read(&mut buf);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn test_mock_socket_stream_write_error() {
        let mut stream = MockSocketStream::new("").with_write_error(io::ErrorKind::BrokenPipe);
        let result = stream.write(b"test");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn test_mock_socket_connector_success() {
        let connector = MockSocketConnector::new("response\n");
        let result = connector.connect(Path::new("/tmp/test.sock"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_mock_socket_connector_connection_refused() {
        let connector = MockSocketConnector::connection_refused();
        let result = connector.connect(Path::new("/tmp/test.sock"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionRefused);
    }

    #[test]
    fn test_mock_socket_connector_not_found() {
        let connector = MockSocketConnector::not_found();
        assert!(!connector.path_exists(Path::new("/tmp/test.sock")));
    }

    #[test]
    fn test_mock_socket_connector_timed_out() {
        let connector = MockSocketConnector::timed_out();
        let result = connector.connect(Path::new("/tmp/test.sock"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    // Command runner tests
    #[test]
    fn test_mock_command_runner_pgrep_running() {
        let runner = MockCommandRunner::new().pgrep_running();
        let output = runner.run("pgrep", &["aka-daemon"]).unwrap();
        assert!(output.success);
        assert_eq!(output.stdout_str().trim(), "12345");
    }

    #[test]
    fn test_mock_command_runner_pgrep_not_running() {
        let runner = MockCommandRunner::new().pgrep_not_running();
        let output = runner.run("pgrep", &["aka-daemon"]).unwrap();
        assert!(!output.success);
        assert!(output.stdout.is_empty());
    }

    #[test]
    fn test_mock_command_runner_systemctl_success() {
        let runner = MockCommandRunner::new().systemctl_success();
        let output = runner.run("systemctl", &["--user", "start", "aka-daemon"]).unwrap();
        assert!(output.success);
    }

    #[test]
    fn test_mock_command_runner_systemctl_failure() {
        let runner = MockCommandRunner::new().systemctl_failure();
        let output = runner.run("systemctl", &["--user", "start", "aka-daemon"]).unwrap();
        assert!(!output.success);
    }

    #[test]
    fn test_mock_command_runner_which_found() {
        let runner = MockCommandRunner::new().which_found("/usr/bin/aka-daemon");
        let output = runner.run("which", &["aka-daemon"]).unwrap();
        assert!(output.success);
        assert_eq!(output.stdout_str().trim(), "/usr/bin/aka-daemon");
    }

    #[test]
    fn test_mock_command_runner_not_mocked() {
        let runner = MockCommandRunner::new();
        let result = runner.run("unknown_command", &[]);
        assert!(result.is_err());
    }

    // File system tests
    #[test]
    fn test_mock_filesystem_read_write() {
        let fs = MockFileSystem::new();
        fs.write(Path::new("/tmp/test.txt"), b"hello").unwrap();
        let contents = fs.read_to_string(Path::new("/tmp/test.txt")).unwrap();
        assert_eq!(contents, "hello");
    }

    #[test]
    fn test_mock_filesystem_with_file() {
        let fs = MockFileSystem::new().with_file("/tmp/test.txt", "contents");
        let contents = fs.read_to_string(Path::new("/tmp/test.txt")).unwrap();
        assert_eq!(contents, "contents");
    }

    #[test]
    fn test_mock_filesystem_exists() {
        let fs = MockFileSystem::new()
            .with_file("/tmp/file.txt", "")
            .with_dir("/tmp/dir");

        assert!(fs.exists(Path::new("/tmp/file.txt")));
        assert!(fs.exists(Path::new("/tmp/dir")));
        assert!(!fs.exists(Path::new("/tmp/nonexistent")));
    }

    #[test]
    fn test_mock_filesystem_not_found() {
        let fs = MockFileSystem::new();
        let result = fs.read_to_string(Path::new("/nonexistent"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn test_mock_filesystem_is_file_is_dir() {
        let fs = MockFileSystem::new()
            .with_file("/tmp/file.txt", "")
            .with_dir("/tmp/dir");

        assert!(fs.is_file(Path::new("/tmp/file.txt")));
        assert!(!fs.is_dir(Path::new("/tmp/file.txt")));
        assert!(fs.is_dir(Path::new("/tmp/dir")));
        assert!(!fs.is_file(Path::new("/tmp/dir")));
    }

    #[test]
    fn test_command_output_from_std_output() {
        // This tests the conversion from std::process::Output
        let output = CommandOutput {
            stdout: b"stdout".to_vec(),
            stderr: b"stderr".to_vec(),
            success: true,
            code: Some(0),
        };
        assert_eq!(output.stdout_str(), "stdout");
        assert_eq!(output.stderr_str(), "stderr");
    }
}
