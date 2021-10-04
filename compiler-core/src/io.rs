pub mod memory;

use crate::error::{Error, FileIoAction, FileKind, Result};
use async_trait::async_trait;
use debug_ignore::DebugIgnore;
use flate2::read::GzDecoder;
use std::{
    fmt::Debug,
    io,
    path::{Path, PathBuf},
};
use tar::Archive;

pub trait Utf8Writer: std::fmt::Write {
    /// A wrapper around `fmt::Write` that has Gleam's error handling.
    fn str_write(&mut self, str: &str) -> Result<()> {
        let res = self.write_str(str);
        self.wrap_result(res)
    }

    fn wrap_result<T, E: std::error::Error>(&self, result: Result<T, E>) -> Result<()> {
        self.convert_err(result.map(|_| ()))
    }

    fn convert_err<T, E: std::error::Error>(&self, result: Result<T, E>) -> Result<T>;
}

impl Utf8Writer for String {
    fn convert_err<T, E: std::error::Error>(&self, result: Result<T, E>) -> Result<T> {
        result.map_err(|error| Error::FileIo {
            action: FileIoAction::WriteTo,
            kind: FileKind::File,
            path: PathBuf::from("<in memory>"),
            err: Some(error.to_string()),
        })
    }
}

pub trait Writer: std::io::Write + Utf8Writer {
    /// A wrapper around `io::Write` that has Gleam's error handling.
    fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let res = std::io::Write::write(self, bytes);
        self.wrap_result(res)
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct OutputFile {
    pub text: String,
    pub path: PathBuf,
}

/// A trait used to read files.
/// Typically we use an implementation that reads from the file system,
/// but in tests and in other places other implementations may be used.
pub trait FileSystemReader {
    fn gleam_files(&self, dir: &Path) -> Box<dyn Iterator<Item = PathBuf>>;
    fn read(&self, path: &Path) -> Result<String, Error>;
    fn reader(&self, path: &Path) -> Result<WrappedReader, Error>;
    fn is_file(&self, path: &Path) -> bool;
    fn is_directory(&self, path: &Path) -> bool;
}

pub trait FileSystemIO: FileSystemWriter + FileSystemReader {}

/// A trait used to write files.
/// Typically we use an implementation that writes to the file system,
/// but in tests and in other places other implementations may be used.
pub trait FileSystemWriter {
    fn writer(&self, path: &Path) -> Result<WrappedWriter, Error>;
}

#[derive(Debug)]
/// A wrapper around a Read implementing object that has Gleam's error handling.
pub struct WrappedReader {
    path: PathBuf,
    inner: DebugIgnore<Box<dyn std::io::Read>>,
}

impl WrappedReader {
    pub fn new(path: &Path, inner: Box<dyn std::io::Read>) -> Self {
        Self {
            path: path.to_path_buf(),
            inner: DebugIgnore(inner),
        }
    }

    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl std::io::Read for WrappedReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.read(buffer)
    }
}

#[derive(Debug)]
/// A wrapper around a Write implementing object that has Gleam's error handling.
pub struct WrappedWriter {
    path: PathBuf,
    inner: DebugIgnore<Box<dyn std::io::Write>>,
}

impl Writer for WrappedWriter {}

impl Utf8Writer for WrappedWriter {
    fn convert_err<T, E: std::error::Error>(&self, result: Result<T, E>) -> Result<T> {
        result.map_err(|error| Error::FileIo {
            action: FileIoAction::WriteTo,
            kind: FileKind::File,
            path: self.path.to_path_buf(),
            err: Some(error.to_string()),
        })
    }
}

impl WrappedWriter {
    pub fn new(path: &Path, inner: Box<dyn std::io::Write>) -> Self {
        Self {
            path: path.to_path_buf(),
            inner: DebugIgnore(inner),
        }
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let result = self.inner.write(bytes);
        self.wrap_result(result)
    }
}

impl<'a> std::io::Write for WrappedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl<'a> std::fmt::Write for WrappedWriter {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.inner
            .write(s.as_bytes())
            .map(|_| ())
            .map_err(|_| std::fmt::Error)
    }
}

#[cfg(test)]
pub mod test {
    use super::*;
    use std::{
        cell::RefCell,
        io::Write,
        rc::Rc,
        sync::mpsc::{self, Receiver, Sender},
    };

    #[derive(Debug, Clone)]
    pub struct FilesChannel(Sender<(PathBuf, InMemoryFile)>);

    impl FilesChannel {
        pub fn new() -> (Self, Receiver<(PathBuf, InMemoryFile)>) {
            let (sender, receiver) = mpsc::channel();
            (Self(sender), receiver)
        }

        pub fn recv_utf8_files(
            receiver: &Receiver<(PathBuf, InMemoryFile)>,
        ) -> Result<Vec<OutputFile>, ()> {
            receiver
                .try_iter()
                .map(|(path, file)| {
                    Ok(OutputFile {
                        path,
                        text: String::from_utf8(file.into_contents()?).map_err(|_| ())?,
                    })
                })
                .collect()
        }
    }

    impl FileSystemWriter for FilesChannel {
        fn writer<'a>(&self, path: &'a Path) -> Result<WrappedWriter, Error> {
            let file = InMemoryFile::new();
            let _ = self.0.send((path.to_path_buf(), file.clone()));
            Ok(WrappedWriter::new(path, Box::new(file)))
        }
    }

    impl FileSystemReader for FilesChannel {
        fn gleam_files(&self, _dir: &Path) -> Box<dyn Iterator<Item = PathBuf>> {
            unimplemented!()
        }

        fn read(&self, _path: &Path) -> Result<String, Error> {
            unimplemented!()
        }

        fn is_file(&self, _path: &Path) -> bool {
            unimplemented!()
        }

        fn reader(&self, _path: &Path) -> Result<WrappedReader, Error> {
            unimplemented!()
        }

        fn is_directory(&self, _path: &Path) -> bool {
            unimplemented!()
        }
    }

    impl FileSystemIO for FilesChannel {}

    #[derive(Debug, Default, Clone)]
    pub struct InMemoryFile {
        contents: Rc<RefCell<Vec<u8>>>,
    }

    impl InMemoryFile {
        pub fn new() -> Self {
            Default::default()
        }

        pub fn into_contents(self) -> Result<Vec<u8>, ()> {
            Rc::try_unwrap(self.contents)
                .map_err(|_| ())
                .map(RefCell::into_inner)
        }
    }

    impl Write for InMemoryFile {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.contents.borrow_mut().write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.contents.borrow_mut().flush()
        }
    }

    impl std::fmt::Write for InMemoryFile {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            self.contents
                .borrow_mut()
                .write(s.as_bytes())
                .map(|_| ())
                .map_err(|_| std::fmt::Error)
        }
    }

    impl Utf8Writer for InMemoryFile {
        fn convert_err<T, E: std::error::Error>(&self, result: Result<T, E>) -> Result<T> {
            result.map_err(|error| Error::FileIo {
                action: FileIoAction::WriteTo,
                kind: FileKind::File,
                path: PathBuf::from("<in memory test file>"),
                err: Some(error.to_string()),
            })
        }
    }

    impl Writer for InMemoryFile {}
}

#[async_trait]
pub trait HttpClient {
    async fn send(&self, request: http::Request<Vec<u8>>)
        -> Result<http::Response<Vec<u8>>, Error>;
}

pub trait TarUnpacker {
    fn io_result_unpack(
        &self,
        path: &Path,
        archive: Archive<GzDecoder<WrappedReader>>,
    ) -> io::Result<()>;

    fn unpack(&self, path: &Path, archive: Archive<GzDecoder<WrappedReader>>) -> Result<()> {
        tracing::trace!(path = ?path, "unpacking tar archive");
        self.io_result_unpack(path, archive)
            .map_err(|e| Error::FileIo {
                action: FileIoAction::WriteTo,
                kind: FileKind::Directory,
                path: path.to_path_buf(),
                err: Some(e.to_string()),
            })
    }
}
