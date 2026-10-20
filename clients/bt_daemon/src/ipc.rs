//! IPC server transport implementations (Single Responsibility Principle).
//!
//! Abstracts platform-specific Unix domain sockets and Windows Named Pipes.

#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixListener;

#[cfg(unix)]
pub struct IpcServer {
    socket_path: String,
}

#[cfg(unix)]
impl IpcServer {
    pub fn new(name: &str) -> Self {
        Self {
            socket_path: format!("/tmp/{}.sock", name),
        }
    }

    pub fn listen<F>(&self, handler: F) -> std::io::Result<()>
    where
        F: Fn(String) -> String + Send + Sync + 'static,
    {
        let _ = std::fs::remove_file(&self.socket_path);
        let listener = UnixListener::bind(&self.socket_path)?;
        let handler = std::sync::Arc::new(handler);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let handler_clone = handler.clone();
                    std::thread::spawn(move || {
                        let mut writer = match stream.try_clone() {
                            Ok(w) => w,
                            Err(_) => return,
                        };
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        while reader.read_line(&mut line).unwrap_or(0) > 0 {
                            let trimmed = line.trim();
                            if !trimmed.is_empty() {
                                let reply = handler_clone(trimmed.to_string());
                                let mut reply_bytes = reply.into_bytes();
                                reply_bytes.push(b'\n');
                                if writer.write_all(&reply_bytes).is_err() {
                                    break;
                                }
                                let _ = writer.flush();
                            }
                            line.clear();
                        }
                    });
                }
                Err(_) => {}
            }
        }
        Ok(())
    }
}

#[cfg(windows)]
use std::ptr;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{PIPE_ACCESS_DUPLEX, ReadFile, WriteFile};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};

#[cfg(windows)]
pub struct IpcServer {
    pipe_name: Vec<u16>,
}

#[cfg(windows)]
impl IpcServer {
    pub fn new(name: &str) -> Self {
        let full_name = format!("\\\\.\\pipe\\{}", name);
        let mut wname: Vec<u16> = full_name.encode_utf16().collect();
        wname.push(0);
        Self { pipe_name: wname }
    }

    pub fn listen<F>(&self, handler: F) -> std::io::Result<()>
    where
        F: Fn(String) -> String + Send + Sync + 'static,
    {
        let handler = std::sync::Arc::new(handler);
        loop {
            let pipe = unsafe {
                CreateNamedPipeW(
                    self.pipe_name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    255,  // max instances
                    4096, // out buffer
                    4096, // in buffer
                    0,    // default timeout
                    ptr::null(),
                )
            };

            if pipe == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }

            let connected = unsafe { ConnectNamedPipe(pipe, ptr::null_mut()) };
            let last_err = if connected == 0 {
                unsafe { windows_sys::Win32::Foundation::GetLastError() }
            } else {
                0
            };
            if connected != 0
                || last_err == 997
                || last_err == windows_sys::Win32::Foundation::ERROR_PIPE_CONNECTED
            {
                let handler_clone = handler.clone();
                let pipe_raw = pipe as usize;
                std::thread::spawn(move || {
                    let h_pipe = pipe_raw as HANDLE;
                    let mut buf = [0u8; 4096];
                    let mut read_bytes = 0u32;
                    let mut request_str = String::new();
                    loop {
                        let ok = unsafe {
                            ReadFile(
                                h_pipe,
                                buf.as_mut_ptr() as *mut _,
                                buf.len() as u32,
                                &mut read_bytes,
                                ptr::null_mut(),
                            )
                        };
                        if ok == 0 || read_bytes == 0 {
                            break;
                        }
                        if let Ok(s) = std::str::from_utf8(&buf[..read_bytes as usize]) {
                            request_str.push_str(s);
                            if request_str.contains('\n') {
                                let mut lines: Vec<&str> = request_str.split('\n').collect();
                                let remaining = lines.pop().unwrap_or("").to_string();
                                for line in lines {
                                    let trimmed = line.trim();
                                    if !trimmed.is_empty() {
                                        let reply = handler_clone(trimmed.to_string());
                                        let mut reply_bytes = reply.into_bytes();
                                        reply_bytes.push(b'\n');
                                        let mut written = 0u32;
                                        unsafe {
                                            WriteFile(
                                                h_pipe,
                                                reply_bytes.as_ptr() as *const _,
                                                reply_bytes.len() as u32,
                                                &mut written,
                                                ptr::null_mut(),
                                            );
                                        }
                                    }
                                }
                                request_str = remaining;
                            }
                        }
                    }
                    unsafe {
                        DisconnectNamedPipe(h_pipe);
                        CloseHandle(h_pipe);
                    }
                });
            } else {
                unsafe {
                    CloseHandle(pipe);
                }
            }
        }
    }
}
