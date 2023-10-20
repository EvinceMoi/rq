use anyhow::Result;
use image::RgbaImage;
use log::{debug, error};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym},
        pointer::{PointerEvent, PointerEventKind, PointerHandler, BTN_LEFT},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{
        slot::{Slot, SlotPool},
        Shm, ShmHandler,
    },
};
use std::{collections::HashMap, time::Instant};
use tiny_skia::{Color, Pixmap, IntRect};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{
        wl_compositor::{self, WlCompositor},
        wl_keyboard::WlKeyboard,
        wl_output::{Transform, WlOutput},
        wl_pointer::WlPointer,
        wl_registry::{self, WlRegistry},
        wl_seat::WlSeat,
        wl_shm,
        wl_surface::WlSurface,
    },
    Connection, Dispatch, Proxy, QueueHandle,
};

use crate::capture;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
struct Pos {
    x: i32,
    y: i32,
}
impl Pos {
    fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

type Region = IntRect;

struct LayerContext {
    layer: LayerSurface,
    region: Region,
    pixmap: Pixmap,
}

#[derive(Default, Debug)]
struct Selection {
    data: Option<(Pos, Pos)>, // (from, to)
    on: bool,
}
impl Selection {
    #[inline]
    pub fn from(&self) -> Option<Pos> {
        self.data.map(|d| d.0)
    }
    #[inline]
    pub fn to(&self) -> Option<Pos> {
        self.data.map(|d| d.1)
    }
    #[inline]
    pub fn set_from(&mut self, from: Pos) {
        let data = self.data.get_or_insert(Default::default());
        data.0 = from;
    }
    #[inline]
    pub fn set_to(&mut self, to: Pos) {
        let data = self.data.get_or_insert(Default::default());
        data.1 = to;
    }
    #[inline]
    pub fn has_value(&self) -> bool {
        self.data.is_some()
    }
    #[inline]
    pub fn reset(&mut self) {
        self.on = false;
        self.data = None;
    }
    #[inline]
    pub fn begin(&mut self, pos: Pos) {
        self.reset();
        self.on = true;
        self.set_from(pos);
        self.set_to(pos);
    }
    #[inline]
    pub fn update(&mut self, pos: Pos) {
        if self.on {
            self.set_to(pos);
        }
    }
    #[inline]
    pub fn end(&mut self, pos: Pos) {
        if self.on {
            self.on = false;
            self.set_to(pos);
        }
    }
    #[inline]
    pub fn to_region(&self) -> Option<Region> {
        self.data.map(|(from, to)| {
            let x = from.x.min(to.x);
            let y = from.y.min(to.y);
            let w = (from.x - to.x).abs() as u32;
            let h = (from.y - to.y).abs() as u32;
            Region::from_xywh(x, y, w, h)
        }).flatten()
    }
}

struct LayerState {
    registry_state: RegistryState,
    shm: Shm,
    output_state: OutputState,
    seat_state: SeatState,

    pool: SlotPool,
    layer: Vec<LayerContext>,
    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,

    exit: bool,
    pos_pressed: Option<Pos>,
    pos_current: Pos, // current pointer postion
    selection: Selection,
    last_draw: Instant,
}
// impl LayerState {
//     pub fn image(&mut self) {
//         self.selection.to_region().map(|region| {
//             self.layer.iter()
//                 .map(|ctx| {
//                     ctx.region.intersect(&region)
//                         // .map(|rect| {
//                         //     ctx.pixmap.clone_rect(rect)
//                         // })
//                         // .flatten()
//                 })
//                 .for_each(|rect| {
//                     debug!("intersection: {:?}", rect);
//                 })
//         });
//     }
// }
impl LayerState {
    pub fn draw(&mut self, qh: &QueueHandle<Self>, surface: &WlSurface) {
        self.last_draw = Instant::now();
        self.layer
            .iter_mut()
            .find(|layer| layer.layer.wl_surface().id().eq(&surface.id()))
            .map(|ctx| {
                let width = ctx.region.width();
                let height = ctx.region.height();
                let (buffer, canvas) = self
                    .pool
                    .create_buffer(width as i32, height as i32, width as i32 * 4, wl_shm::Format::Argb8888)
                    .expect("create buffer");

                ctx.pixmap.fill(Color::from_rgba8(0x64, 0x64, 0x64, 0x80)); // bgra
                if self.selection.has_value() {
                    use tiny_skia::*;
                    let paint = {
                        let mut paint = Paint::default();
                        paint.set_color_rgba8(0, 0, 0, 0x00);
                        paint.blend_mode = BlendMode::Source;
                        paint
                    };
                    let from = self.selection.from().unwrap();
                    let to = self.selection.to().unwrap();
                    let rect = Rect::from_points(&[
                        Point {
                            x: from.x as f32,
                            y: from.y as f32,
                        },
                        Point {
                            x: to.x as f32,
                            y: to.y as f32,
                        },
                    ])
                    .unwrap();
                    if rect.height() > 0. && rect.width() > 0. {
                        ctx.pixmap.fill_rect(
                            rect,
                            &paint,
                            Transform::from_translate(
                                -ctx.region.x() as f32,
                                -ctx.region.y() as f32,
                            ),
                            None,
                        );
                    }
                }

                canvas.copy_from_slice(ctx.pixmap.data());

                surface.damage_buffer(0, 0, width as i32, height as i32);

                buffer.attach_to(surface).expect("buffer attach");

                // request redraw with current buffer and call frame callback
                surface.frame(qh, surface.clone());

                surface.commit();
            });
    }
}

delegate_registry!(LayerState);
impl ProvidesRegistryState for LayerState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![SeatState];
}
delegate_compositor!(LayerState);
impl CompositorHandler for LayerState {
    fn scale_factor_changed(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &WlSurface,
        new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &WlSurface,
        new_transform: Transform,
    ) {
    }

    fn frame(&mut self, conn: &Connection, qh: &QueueHandle<Self>, surface: &WlSurface, time: u32) {
        // frame callback
        self.selection.update(self.pos_current);

        {
            let fps = 60;
            let interval = 1000 / fps;
            let now = Instant::now();
            let elapsed_ms = now.duration_since(self.last_draw).as_millis();
            if elapsed_ms < interval {
                std::thread::sleep(std::time::Duration::from_millis(
                    (interval - elapsed_ms) as u64,
                ));
            }
            self.draw(qh, surface);
        }
    }
}
delegate_output!(LayerState);
impl OutputHandler for LayerState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, conn: &Connection, qh: &QueueHandle<Self>, output: WlOutput) {
        // self.outputs.push(output);
    }

    fn update_output(&mut self, conn: &Connection, qh: &QueueHandle<Self>, output: WlOutput) {}

    fn output_destroyed(&mut self, conn: &Connection, qh: &QueueHandle<Self>, output: WlOutput) {}
}

delegate_shm!(LayerState);
impl ShmHandler for LayerState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
delegate_layer!(LayerState);
impl LayerShellHandler for LayerState {
    fn closed(&mut self, conn: &Connection, qh: &QueueHandle<Self>, layer: &LayerSurface) {
        // todo!()
    }

    fn configure(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        serial: u32,
    ) {
        // start firer draw here
        self.draw(qh, layer.wl_surface());
    }
}
delegate_seat!(LayerState);
impl SeatHandler for LayerState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, conn: &Connection, qh: &QueueHandle<Self>, seat: WlSeat) {}

    fn new_capability(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            let keyboard = self
                .seat_state
                .get_keyboard(qh, &seat, None)
                .expect("Failed to create keyboard");
            self.keyboard = Some(keyboard);
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            let pointer = self
                .seat_state
                .get_pointer(qh, &seat)
                .expect("Failed to create pointer");
            self.pointer = Some(pointer);
        }
    }

    fn remove_capability(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_some() {
            println!("Unset keyboard capability");
            self.keyboard.take().unwrap().release();
        }

        if capability == Capability::Pointer && self.pointer.is_some() {
            println!("Unset pointer capability");
            self.pointer.take().unwrap().release();
        }
    }

    fn remove_seat(&mut self, conn: &Connection, qh: &QueueHandle<Self>, seat: WlSeat) {}
}

delegate_keyboard!(LayerState);
impl KeyboardHandler for LayerState {
    fn enter(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        keyboard: &WlKeyboard,
        surface: &WlSurface,
        serial: u32,
        raw: &[u32],
        keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        surface: &wayland_client::protocol::wl_surface::WlSurface,
        serial: u32,
    ) {
    }

    fn press_key(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        keyboard: &WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        if event.keysym == Keysym::Escape {
            self.exit = true;
        }
    }

    fn release_key(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        serial: u32,
        event: smithay_client_toolkit::seat::keyboard::KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        serial: u32,
        modifiers: smithay_client_toolkit::seat::keyboard::Modifiers,
    ) {
    }
}
delegate_pointer!(LayerState);
impl PointerHandler for LayerState {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        pointer: &WlPointer,
        events: &[PointerEvent],
    ) {
        use PointerEventKind::*;
        for event in events {
            let region = self
                .layer
                .iter()
                .find(|layer| layer.layer.wl_surface().id().eq(&event.surface.id()))
                .map(|ctx| ctx.region)
                .unwrap();
            let pos = Pos {
                x: event.position.0.floor() as i32 + region.x(),
                y: event.position.1.floor() as i32 + region.y(),
            };
            self.pos_current = pos;

            if let Some(pressed_pos) = self.pos_pressed {
                if !pos.eq(&pressed_pos) {
                    self.selection.begin(pressed_pos);
                }
            }

            match event.kind {
                Enter { .. } => {}
                Leave { .. } => {}
                Press { button, .. } => {
                    event.position;
                    if button & BTN_LEFT > 0 {
                        self.pos_pressed = Some(pos);
                    }
                }
                Release { button, .. } => {
                    event.position;
                    if button & BTN_LEFT > 0 {
                        self.pos_pressed = None;
                        self.selection.end(pos);
                        self.exit = true;
                    }
                }
                _ => {}
            }
        }
    }
}

pub fn run() -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<LayerState>(&conn)?;
    let qh = event_queue.handle();

    let registry_state = RegistryState::new(&globals);
    let output_state = OutputState::new(&globals, &qh);

    let compositor = CompositorState::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let seat_state = SeatState::new(&globals, &qh);
    let pool = SlotPool::new(1920 * 1080 * 4, &shm).expect("failed to create pool");

    let mut layer_state = LayerState {
        registry_state,
        shm,
        output_state,
        seat_state,

        pool,
        layer: Vec::new(),
        keyboard: None,
        pointer: None,

        exit: false,
        pos_pressed: None,
        pos_current: Default::default(),
        selection: Default::default(),
        last_draw: Instant::now(),
    };
    // get output
    event_queue.roundtrip(&mut layer_state)?;

    // init layer
    layer_state.output_state.outputs().for_each(|output| {
        let (name, region) = layer_state
            .output_state
            .info(&output)
            .map(|info| {
                let region = Region::from_xywh(
                    info.logical_position.unwrap().0,
                    info.logical_position.unwrap().1,
                    info.logical_size.unwrap().0 as u32,
                    info.logical_size.unwrap().1 as u32,
                ).unwrap();
                (info.name, region)
            })
            .unwrap();
        let surface = compositor.create_surface(&qh);
        let layer =
            layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, name, Some(&output));
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_size(region.width(), region.height());
        layer.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
        layer.commit();
        let pixmap = Pixmap::new(region.width(), region.height()).unwrap();
        layer_state.layer.push(LayerContext {
            layer,
            region,
            pixmap,
        });
    });
    event_queue.roundtrip(&mut layer_state)?;

    loop {
        event_queue.blocking_dispatch(&mut layer_state)?;
        if layer_state.exit {
            break;
        }
    }
    let region = layer_state.selection.to_region();
    debug!("got region: {:?}", region);

    region.map(|area| {
        let capture = futures::executor::block_on(async {
            capture::area(area.x(), area.y(), area.width(), area.height()).await
        });
        match capture {
            Ok(raw) => Some(raw),
            Err(e) => {
                error!("capture error: {e}");
                None
            },
        }
    })
    .flatten()
    .map(|raw| {
        RgbaImage::from_vec(raw.width, raw.height, raw.buf)
    })
    .flatten()
    .map(|img| {
        let decoder = bardecoder::default_decoder();
        for result in decoder.decode(&img) {
            match result {
                Ok(decoded) => debug!("decoded: {decoded}"),
                Err(_) => {},
            }
        }
    });

    Ok(())
}
