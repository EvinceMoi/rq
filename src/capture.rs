#![allow(dead_code)]
use anyhow::Result;
use libc::{self, c_int};
use std::{
    collections::HashMap,
    fs::File,
    future::Future,
    io::{self, Read},
    os::fd::FromRawFd,
};
use zbus::{
    dbus_proxy,
    zvariant::{Fd, OwnedValue, Value},
    Connection,
};

pub struct RawCaptured {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub buf: Vec<u8>,
}

#[dbus_proxy(
    default_service = "org.kde.KWin.ScreenShot2",
    interface = "org.kde.KWin.ScreenShot2",
    default_path = "/org/kde/KWin/ScreenShot2"
)]
trait KWin {
    /// options:
    ///     include-decoration: bool
    ///     include-cursor: bool
    ///     native-resolution: bool
    fn capture_area(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        options: HashMap<&str, &Value<'_>>,
        pipe: Fd,
    ) -> zbus::Result<HashMap<String, OwnedValue>>;

    fn capture_active_screen(
        &self,
        options: HashMap<&str, &Value<'_>>,
        pipe: Fd,
    ) -> zbus::Result<HashMap<String, OwnedValue>>;

    fn capture_screen(
        &self,
        name: &str,
        options: HashMap<&str, &Value<'_>>,
        pipe: Fd,
    ) -> zbus::Result<HashMap<String, OwnedValue>>;

    fn capture_active_window(
        &self,
        options: HashMap<&str, &Value<'_>>,
        pipe: Fd,
    ) -> zbus::Result<HashMap<String, OwnedValue>>;

    fn capture_window(
        &self,
        handle: &str,
        options: HashMap<&str, &Value<'_>>,
        pipe: Fd,
    ) -> zbus::Result<HashMap<String, OwnedValue>>;

    fn capture_workspace(
        &self,
        options: HashMap<&str, &Value<'_>>,
        pipe: Fd,
    ) -> zbus::Result<HashMap<String, OwnedValue>>;

    fn capture_interactive(
        &self,
        kind: u32,
        options: HashMap<&str, &Value<'_>>,
        pipe: Fd,
    ) -> zbus::Result<HashMap<String, OwnedValue>>;
}

async fn with_kwin<F, Fut>(f: F) -> Result<RawCaptured>
where
    F: FnOnce(Connection, Fd) -> Fut,
    Fut: Future<Output = zbus::Result<HashMap<String, OwnedValue>>>,
{
    let conn = Connection::session().await?;
    let mut fds: [c_int; 2] = [0; 2];
    let res = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if res != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let captured = f(conn, fds[1].into()).await?;
    unsafe {
        libc::close(fds[1]);
    }

    fn extract<'a, T>(captured: &'a HashMap<String, OwnedValue>, key: &str, default: T) -> T
    where
        T: Clone,
        &'a T: TryFrom<&'a Value<'a>> + 'a,
    {
        captured
            .get(key)
            .map_or(&default, |v| v.downcast_ref().unwrap_or(&default))
            .to_owned()
    }

    let _oformat: u32 = extract(&captured, "format", 0);
    let owidth: u32 = extract(&captured, "width", 0);
    let oheight: u32 = extract(&captured, "height", 0);
    let _ostride: u32 = extract(&captured, "stride", 0);
    let oscale: f64 = extract(&captured, "scale", 0.);

    // read to buf
    let mut f = unsafe { File::from_raw_fd(fds[0]) };
    let data_size = (owidth * 4 * oheight) as usize;
    let mut buf = Vec::with_capacity(data_size);
    {
        let _size = f.read_to_end(&mut buf)?;
    }

    let raw = RawCaptured {
        width: owidth,
        height: oheight,
        scale: oscale,
        buf: buf
            .chunks_exact(4)
            .flat_map(|bgra| [bgra[2], bgra[1], bgra[0], bgra[3]])
            .collect::<Vec<u8>>(),
    };

    Ok(raw)
}

pub async fn workspace() -> Result<RawCaptured> {
    let native_resolution = Value::from(true);
    let options = HashMap::from([("native-resolution", &native_resolution)]);
    let img = with_kwin(|conn, fd| async move {
        let proxy = KWinProxy::new(&conn).await?;
        proxy.capture_workspace(options, fd).await
    })
    .await?;
    Ok(img)
}

pub async fn area(x: i32, y: i32, w: u32, h: u32) -> Result<RawCaptured> {
    let native_resolution = Value::from(true);
    let options = HashMap::from([("native-resolution", &native_resolution)]);
    let img = with_kwin(|conn, fd| async move {
        let proxy = KWinProxy::new(&conn).await?;
        proxy.capture_area(x, y, w, h, options, fd).await
    })
    .await?;
    Ok(img)
}

pub async fn screen(name: &str) -> Result<RawCaptured> {
    let native_resolution = Value::from(true);
    let options = HashMap::from([("native-resolution", &native_resolution)]);
    let img = with_kwin(|conn, fd| async move {
        let proxy = KWinProxy::new(&conn).await?;
        proxy.capture_screen(name, options, fd).await
    })
    .await?;
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use image::{ImageBuffer, Rgba};

    #[test]
    fn test_capture_screen() {
        block_on(async {
            let captured = screen("DP-1").await;
            match captured {
                Ok(img) => {
                    let img: Option<ImageBuffer<Rgba<u8>, Vec<u8>>> =
                        ImageBuffer::from_vec(img.width, img.height, img.buf);
                    match img {
                        Some(img) => {
                            let _ = img.save("./screen.jpeg");
                        }
                        None => {
                            eprint!("no image");
                        }
                    }
                }
                Err(err) => {
                    eprintln!("error: {err:?}");
                }
            }
        })
    }
}
