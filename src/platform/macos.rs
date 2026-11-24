//! macOS: окно через Cocoa (Objective-C runtime), вывод кадра через CoreGraphics.
//!
//! Классы `AlphaCadView`, `AlphaCadWindowDelegate` и `AlphaCadAppDelegate` создаются во
//! время выполнения. Главный цикл — ручной (`nextEventMatchingMask:`), чтобы
//! перерисовывать окно только при событиях и по таймерам анимаций.
#![cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports, unused_macros))]
#![allow(non_upper_case_globals, non_snake_case, clippy::missing_safety_doc)]

use super::{Event, Filter, FrameOutput, Handler};
use crate::ui::{Cursor, Key, Mods, MouseButton};
use std::cell::{Cell, RefCell};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::ptr::null_mut;
use std::time::{Duration, Instant};

type Id = *mut c_void;
type Sel = *const c_void;
type Class = *mut c_void;
type Bool = i8;
const YES: Bool = 1;
const NO: Bool = 0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSSize {
    pub w: f64,
    pub h: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
}

#[cfg_attr(target_os = "macos", link(name = "objc"))]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    #[cfg(target_arch = "x86_64")]
    fn objc_msgSend_stret();
    fn objc_allocateClassPair(superclass: Class, name: *const c_char, extra: usize) -> Class;
    fn objc_registerClassPair(cls: Class);
    fn class_addMethod(cls: Class, name: Sel, imp: *const c_void, types: *const c_char) -> Bool;
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

#[cfg_attr(target_os = "macos", link(name = "AppKit", kind = "framework"))]
extern "C" {}

#[cfg_attr(target_os = "macos", link(name = "Foundation", kind = "framework"))]
extern "C" {
    static NSDefaultRunLoopMode: Id;
}

#[cfg_attr(target_os = "macos", link(name = "CoreGraphics", kind = "framework"))]
extern "C" {
    static kCGColorSpaceSRGB: *const c_void;
    fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut c_void;
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGDataProviderCreateWithData(info: *mut c_void, data: *const c_void, size: usize, release: *const c_void) -> *mut c_void;
    fn CGDataProviderRelease(p: *mut c_void);
    fn CGImageCreate(w: usize, h: usize, bpc: usize, bpp: usize, bpr: usize, cs: *mut c_void, info: u32, provider: *mut c_void, decode: *const f64, interpolate: bool, intent: i32) -> *mut c_void;
    fn CGImageRelease(img: *mut c_void);
    fn CGContextDrawImage(ctx: *mut c_void, rect: NSRect, img: *mut c_void);
    fn CGContextSetInterpolationQuality(ctx: *mut c_void, q: i32);
}

/// Вызов метода Objective-C: `msg!(obj, "sel:with:", a => T1, b => T2; Ret)`.
macro_rules! msg {
    ($obj:expr, $sel:literal $(, $arg:expr => $t:ty)* ; $ret:ty) => {{
        #[allow(clippy::missing_transmute_annotations)]
        let f: unsafe extern "C" fn(Id, Sel $(, $t)*) -> $ret = std::mem::transmute(objc_msgSend as *const c_void);
        f($obj as Id, sel_registerName(concat!($sel, "\0").as_ptr() as *const c_char) $(, $arg)*)
    }};
}

unsafe fn cls(name: &str) -> Class {
    let c = CString::new(name).unwrap();
    objc_getClass(c.as_ptr())
}

unsafe fn sel(name: &str) -> Sel {
    let c = CString::new(name).unwrap();
    sel_registerName(c.as_ptr())
}

/// Метод, возвращающий NSRect (на x86_64 — через `objc_msgSend_stret`).
unsafe fn msg_rect(obj: Id, name: &str) -> NSRect {
    #[cfg(target_arch = "x86_64")]
    {
        let f: unsafe extern "C" fn(*mut NSRect, Id, Sel) = std::mem::transmute(objc_msgSend_stret as *const c_void);
        let mut r = NSRect::default();
        f(&mut r, obj, sel(name));
        r
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let f: unsafe extern "C" fn(Id, Sel) -> NSRect = std::mem::transmute(objc_msgSend as *const c_void);
        f(obj, sel(name))
    }
}

unsafe fn nsstring(s: &str) -> Id {
    let c = CString::new(s.replace('\0', "")).unwrap();
    msg!(cls("NSString"), "stringWithUTF8String:", c.as_ptr() => *const c_char; Id)
}

unsafe fn rust_string(s: Id) -> String {
    if s.is_null() {
        return String::new();
    }
    let p: *const c_char = msg!(s, "UTF8String"; *const c_char);
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

unsafe fn nsarray(items: &[Id]) -> Id {
    let arr: Id = msg!(cls("NSMutableArray"), "array"; Id);
    for &i in items {
        msg!(arr, "addObject:", i => Id; ());
    }
    arr
}

unsafe fn array_items(arr: Id) -> Vec<Id> {
    if arr.is_null() {
        return Vec::new();
    }
    let n: usize = msg!(arr, "count"; usize);
    (0..n).map(|i| msg!(arr, "objectAtIndex:", i => usize; Id)).collect()
}

struct Present {
    pixels: Vec<u32>,
    w: usize,
    h: usize,
}

thread_local! {
    static APP: RefCell<Option<Box<dyn Handler>>> = RefCell::new(None);
    static EVENTS: RefCell<Vec<Event>> = RefCell::new(Vec::new());
    static PRESENT: RefCell<Present> = RefCell::new(Present { pixels: Vec::new(), w: 0, h: 0 });
    static VIEW: Cell<Id> = Cell::new(null_mut());
    static WINDOW: Cell<Id> = Cell::new(null_mut());
    static CURSOR: Cell<Cursor> = Cell::new(Cursor::Arrow);
    static TITLE: RefCell<String> = RefCell::new(String::new());
    static WAKE: Cell<Option<Instant>> = Cell::new(None);
    static DIRTY: Cell<bool> = Cell::new(true);
    static QUIT: Cell<bool> = Cell::new(false);
    static COLORSPACE: Cell<*mut c_void> = Cell::new(null_mut());
    static PENDING_FILES: RefCell<Vec<PathBuf>> = RefCell::new(Vec::new());
}

fn push(e: Event) {
    EVENTS.with(|q| q.borrow_mut().push(e));
    DIRTY.with(|d| d.set(true));
}

unsafe fn backing_scale() -> f64 {
    let w = WINDOW.with(|w| w.get());
    if w.is_null() {
        return 1.0;
    }
    let s: f64 = msg!(w, "backingScaleFactor"; f64);
    if s > 0.0 {
        s
    } else {
        1.0
    }
}

unsafe fn send_resize() {
    let v = VIEW.with(|v| v.get());
    if v.is_null() {
        return;
    }
    let b = msg_rect(v, "bounds");
    let s = backing_scale();
    let w = ((b.size.w * s).round() as usize).max(1);
    let h = ((b.size.h * s).round() as usize).max(1);
    push(Event::Resize { width: w, height: h, scale: s as f32 });
}

fn mods_from(flags: u64) -> Mods {
    Mods { shift: flags & (1 << 17) != 0, ctrl: flags & (1 << 18) != 0, alt: flags & (1 << 19) != 0, meta: flags & (1 << 20) != 0 }
}

unsafe fn event_mods(ev: Id) -> Mods {
    let f: u64 = msg!(ev, "modifierFlags"; u64);
    mods_from(f)
}

/// Позиция мыши события в пикселях кадра (начало — левый верхний угол).
unsafe fn event_pos(ev: Id) -> (f32, f32) {
    let v = VIEW.with(|v| v.get());
    let loc: NSPoint = msg!(ev, "locationInWindow"; NSPoint);
    let p: NSPoint = msg!(v, "convertPoint:fromView:", loc => NSPoint, null_mut() => Id; NSPoint);
    let b = msg_rect(v, "bounds");
    let s = backing_scale();
    ((p.x * s) as f32, ((b.size.h - p.y) * s) as f32)
}

fn map_keycode(code: u16) -> Option<Key> {
    let letter = |c: char| Some(Key::Char(c));
    match code {
        0x00 => letter('a'),
        0x01 => letter('s'),
        0x02 => letter('d'),
        0x03 => letter('f'),
        0x04 => letter('h'),
        0x05 => letter('g'),
        0x06 => letter('z'),
        0x07 => letter('x'),
        0x08 => letter('c'),
        0x09 => letter('v'),
        0x0B => letter('b'),
        0x0C => letter('q'),
        0x0D => letter('w'),
        0x0E => letter('e'),
        0x0F => letter('r'),
        0x10 => letter('y'),
        0x11 => letter('t'),
        0x12 | 0x53 => letter('1'),
        0x13 | 0x54 => letter('2'),
        0x14 | 0x55 => letter('3'),
        0x15 | 0x56 => letter('4'),
        0x17 | 0x57 => letter('5'),
        0x16 | 0x58 => letter('6'),
        0x1A | 0x59 => letter('7'),
        0x1C | 0x5B => letter('8'),
        0x19 | 0x5C => letter('9'),
        0x1D | 0x52 => letter('0'),
        0x18 | 0x45 => letter('+'),
        0x1B | 0x4E => letter('-'),
        0x1F => letter('o'),
        0x20 => letter('u'),
        0x22 => letter('i'),
        0x23 => letter('p'),
        0x25 => letter('l'),
        0x26 => letter('j'),
        0x28 => letter('k'),
        0x2D => letter('n'),
        0x2E => letter('m'),
        0x24 | 0x4C => Some(Key::Enter),
        0x30 => Some(Key::Tab),
        0x31 => Some(Key::Space),
        0x33 => Some(Key::Backspace),
        0x35 => Some(Key::Escape),
        0x75 => Some(Key::Delete),
        0x73 => Some(Key::Home),
        0x77 => Some(Key::End),
        0x74 => Some(Key::PageUp),
        0x79 => Some(Key::PageDown),
        0x7B => Some(Key::Left),
        0x7C => Some(Key::Right),
        0x7D => Some(Key::Down),
        0x7E => Some(Key::Up),
        0x7A => Some(Key::F(1)),
        0x78 => Some(Key::F(2)),
        0x63 => Some(Key::F(3)),
        0x76 => Some(Key::F(4)),
        0x60 => Some(Key::F(5)),
        0x61 => Some(Key::F(6)),
        0x62 => Some(Key::F(7)),
        0x64 => Some(Key::F(8)),
        0x65 => Some(Key::F(9)),
        0x6D => Some(Key::F(10)),
        0x67 => Some(Key::F(11)),
        0x6F => Some(Key::F(12)),
        _ => None,
    }
}

unsafe fn apply_cursor(c: Cursor) {
    let cur: Id = match c {
        Cursor::Arrow => msg!(cls("NSCursor"), "arrowCursor"; Id),
        Cursor::Hand => msg!(cls("NSCursor"), "pointingHandCursor"; Id),
        Cursor::Text => msg!(cls("NSCursor"), "IBeamCursor"; Id),
        Cursor::Crosshair => msg!(cls("NSCursor"), "crosshairCursor"; Id),
        Cursor::Move => msg!(cls("NSCursor"), "openHandCursor"; Id),
    };
    if !cur.is_null() {
        msg!(cur, "set"; ());
    }
}

/// Передать события приложению и построить кадр. `false` — приложение занято.
unsafe fn pump_frame() -> bool {
    let done = APP.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else { return None };
        let app = guard.as_mut()?;
        let files: Vec<PathBuf> = PENDING_FILES.with(|p| std::mem::take(&mut *p.borrow_mut()));
        if !files.is_empty() {
            app.event(Event::DropFiles(files));
        }
        let events: Vec<Event> = EVENTS.with(|q| std::mem::take(&mut *q.borrow_mut()));
        for e in events {
            app.event(e);
        }
        DIRTY.with(|d| d.set(false));
        let out: FrameOutput = app.frame();
        if EVENTS.with(|q| !q.borrow().is_empty()) {
            DIRTY.with(|d| d.set(true));
        }
        let fb = app.framebuffer();
        PRESENT.with(|p| {
            let mut p = p.borrow_mut();
            p.w = fb.width;
            p.h = fb.height;
            p.pixels.clear();
            p.pixels.extend_from_slice(&fb.color);
        });
        Some(out)
    });
    let Some(out) = done else { return false };
    let view = VIEW.with(|v| v.get());
    let window = WINDOW.with(|w| w.get());
    if !view.is_null() {
        msg!(view, "setNeedsDisplay:", YES => Bool; ());
        msg!(view, "displayIfNeeded"; ());
    }
    TITLE.with(|t| {
        let mut t = t.borrow_mut();
        if *t != out.title && !window.is_null() {
            *t = out.title.clone();
            msg!(window, "setTitle:", nsstring(&out.title) => Id; ());
        }
    });
    if CURSOR.with(|c| c.get()) != out.cursor {
        CURSOR.with(|c| c.set(out.cursor));
        apply_cursor(out.cursor);
    }
    WAKE.with(|w| w.set(out.wake_ms.map(|ms| Instant::now() + Duration::from_millis(ms))));
    if out.quit {
        QUIT.with(|q| q.set(true));
    }
    true
}

// ---------------------------------------------------------------------------
// Методы классов
// ---------------------------------------------------------------------------

extern "C" fn view_draw_rect(this: Id, _: Sel, _dirty: NSRect) {
    unsafe {
        let gctx: Id = msg!(cls("NSGraphicsContext"), "currentContext"; Id);
        if gctx.is_null() {
            return;
        }
        let cg: *mut c_void = msg!(gctx, "CGContext"; *mut c_void);
        if cg.is_null() {
            return;
        }
        let bounds = msg_rect(this, "bounds");
        PRESENT.with(|p| {
            let p = p.borrow();
            if p.w == 0 || p.h == 0 || p.pixels.len() < p.w * p.h {
                return;
            }
            let mut cs = COLORSPACE.with(|c| c.get());
            if cs.is_null() {
                cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
                if cs.is_null() {
                    cs = CGColorSpaceCreateDeviceRGB();
                }
                COLORSPACE.with(|c| c.set(cs));
            }
            let bytes = p.w * p.h * 4;
            let provider = CGDataProviderCreateWithData(null_mut(), p.pixels.as_ptr() as *const c_void, bytes, std::ptr::null());
            // kCGBitmapByteOrder32Little | kCGImageAlphaNoneSkipFirst
            let img = CGImageCreate(p.w, p.h, 8, 32, p.w * 4, cs, (2 << 12) | 6, provider, std::ptr::null(), false, 0);
            if !img.is_null() {
                CGContextSetInterpolationQuality(cg, 1);
                CGContextDrawImage(cg, NSRect { origin: NSPoint { x: 0.0, y: 0.0 }, size: bounds.size }, img);
                CGImageRelease(img);
            }
            CGDataProviderRelease(provider);
        });
    }
}

extern "C" fn yes(_: Id, _: Sel) -> Bool {
    YES
}

extern "C" fn yes1(_: Id, _: Sel, _: Id) -> Bool {
    YES
}

unsafe fn mouse(ev: Id, button: Option<(MouseButton, bool)>) {
    let (x, y) = event_pos(ev);
    push(Event::Modifiers(event_mods(ev)));
    push(Event::MouseMove { x, y });
    if let Some((b, down)) = button {
        push(Event::MouseButton { button: b, down });
    }
}

extern "C" fn on_mouse_down(_: Id, _: Sel, ev: Id) {
    unsafe {
        // Ctrl+клик — правая кнопка (привычно для macOS с однокнопочной мышью).
        let m = event_mods(ev);
        let b = if m.ctrl && !m.meta { MouseButton::Right } else { MouseButton::Left };
        mouse(ev, Some((b, true)));
        CTRL_CLICK.with(|c| c.set(b == MouseButton::Right));
    }
}

thread_local! {
    static CTRL_CLICK: Cell<bool> = Cell::new(false);
}

extern "C" fn on_mouse_up(_: Id, _: Sel, ev: Id) {
    unsafe {
        let b = if CTRL_CLICK.with(|c| c.replace(false)) { MouseButton::Right } else { MouseButton::Left };
        mouse(ev, Some((b, false)));
    }
}

extern "C" fn on_right_down(_: Id, _: Sel, ev: Id) {
    unsafe { mouse(ev, Some((MouseButton::Right, true))) }
}

extern "C" fn on_right_up(_: Id, _: Sel, ev: Id) {
    unsafe { mouse(ev, Some((MouseButton::Right, false))) }
}

extern "C" fn on_other_down(_: Id, _: Sel, ev: Id) {
    unsafe { mouse(ev, Some((MouseButton::Middle, true))) }
}

extern "C" fn on_other_up(_: Id, _: Sel, ev: Id) {
    unsafe { mouse(ev, Some((MouseButton::Middle, false))) }
}

extern "C" fn on_mouse_moved(_: Id, _: Sel, ev: Id) {
    unsafe {
        mouse(ev, None);
        apply_cursor(CURSOR.with(|c| c.get()));
    }
}

extern "C" fn on_mouse_exited(_: Id, _: Sel, _ev: Id) {
    push(Event::MouseLeave);
}

extern "C" fn on_scroll(_: Id, _: Sel, ev: Id) {
    unsafe {
        let mut dy: f64 = msg!(ev, "scrollingDeltaY"; f64);
        let precise: Bool = msg!(ev, "hasPreciseScrollingDeltas"; Bool);
        if precise != NO {
            dy /= 12.0;
        }
        if dy != 0.0 {
            let (x, y) = event_pos(ev);
            push(Event::MouseMove { x, y });
            push(Event::Modifiers(event_mods(ev)));
            push(Event::Wheel { delta: dy as f32 });
        }
    }
}

extern "C" fn on_magnify(_: Id, _: Sel, ev: Id) {
    unsafe {
        let m: f64 = msg!(ev, "magnification"; f64);
        let (x, y) = event_pos(ev);
        push(Event::MouseMove { x, y });
        push(Event::Wheel { delta: (m * 6.0) as f32 });
    }
}

extern "C" fn on_key_down(_: Id, _: Sel, ev: Id) {
    unsafe {
        let code: u16 = msg!(ev, "keyCode"; u16);
        let m = event_mods(ev);
        if let Some(k) = map_keycode(code) {
            push(Event::Key { key: k, mods: m, down: true });
        }
        if !m.meta && !m.ctrl {
            let chars: Id = msg!(ev, "characters"; Id);
            let s = rust_string(chars);
            let s: String = s.chars().filter(|c| !c.is_control() && !('\u{F700}'..='\u{F8FF}').contains(c)).collect();
            if !s.is_empty() {
                push(Event::Text(s));
            }
        }
    }
}

extern "C" fn on_key_up(_: Id, _: Sel, ev: Id) {
    unsafe {
        let code: u16 = msg!(ev, "keyCode"; u16);
        if let Some(k) = map_keycode(code) {
            push(Event::Key { key: k, mods: event_mods(ev), down: false });
        }
    }
}

extern "C" fn on_flags_changed(_: Id, _: Sel, ev: Id) {
    unsafe { push(Event::Modifiers(event_mods(ev))) }
}

/// Cmd+клавиши приходят сюда раньше `keyDown:` — передаём их приложению,
/// кроме Cmd+Q/Cmd+H (их обрабатывает меню).
extern "C" fn on_key_equivalent(this: Id, _: Sel, ev: Id) -> Bool {
    unsafe {
        let code: u16 = msg!(ev, "keyCode"; u16);
        let m = event_mods(ev);
        if m.meta && (code == 0x0C || code == 0x04) {
            return NO;
        }
        if m.meta || m.ctrl {
            let ty: u64 = msg!(ev, "type"; u64);
            if ty == 10 {
                // NSEventTypeKeyDown
                on_key_down(this, null_mut::<c_void>() as Sel, ev);
                return YES;
            }
        }
        NO
    }
}

extern "C" fn on_backing_changed(_: Id, _: Sel) {
    unsafe {
        send_resize();
    }
}

extern "C" fn dragging_entered(_: Id, _: Sel, _info: Id) -> usize {
    1 // NSDragOperationCopy
}

extern "C" fn perform_drag(_: Id, _: Sel, info: Id) -> Bool {
    unsafe {
        let pb: Id = msg!(info, "draggingPasteboard"; Id);
        let classes = nsarray(&[cls("NSURL")]);
        let urls: Id = msg!(pb, "readObjectsForClasses:options:", classes => Id, null_mut() => Id; Id);
        let mut files = Vec::new();
        for u in array_items(urls) {
            let path: Id = msg!(u, "path"; Id);
            let s = rust_string(path);
            if !s.is_empty() {
                files.push(PathBuf::from(s));
            }
        }
        if files.is_empty() {
            return NO;
        }
        push(Event::DropFiles(files));
        YES
    }
}

extern "C" fn window_should_close(_: Id, _: Sel, _sender: Id) -> Bool {
    push(Event::CloseRequested);
    unsafe {
        pump_frame();
    }
    NO
}

extern "C" fn window_did_resize(_: Id, _: Sel, _n: Id) {
    unsafe {
        send_resize();
        pump_frame();
    }
}

extern "C" fn app_should_terminate(_: Id, _: Sel, _app: Id) -> usize {
    push(Event::CloseRequested);
    0 // NSTerminateCancel — решение о выходе принимает приложение
}

extern "C" fn app_open_files(_: Id, _: Sel, app: Id, files: Id) {
    unsafe {
        let paths: Vec<PathBuf> = array_items(files).into_iter().map(|s| PathBuf::from(rust_string(s))).filter(|p| !p.as_os_str().is_empty()).collect();
        PENDING_FILES.with(|p| p.borrow_mut().extend(paths));
        DIRTY.with(|d| d.set(true));
        msg!(app, "replyToOpenOrPrint:", 0usize => usize; ());
    }
}

unsafe fn add(c: Class, name: &str, imp: *const c_void, types: &str) {
    let t = CString::new(types).unwrap();
    class_addMethod(c, sel(name), imp, t.as_ptr());
}

unsafe fn register_classes() -> (Class, Class, Class) {
    let view = objc_allocateClassPair(cls("NSView"), c"AlphaCadView".as_ptr(), 0);
    add(view, "drawRect:", view_draw_rect as *const c_void, "v@:{CGRect={CGPoint=dd}{CGSize=dd}}");
    add(view, "isOpaque", yes as *const c_void, "c@:");
    add(view, "acceptsFirstResponder", yes as *const c_void, "c@:");
    add(view, "acceptsFirstMouse:", yes1 as *const c_void, "c@:@");
    add(view, "mouseDown:", on_mouse_down as *const c_void, "v@:@");
    add(view, "mouseUp:", on_mouse_up as *const c_void, "v@:@");
    add(view, "mouseDragged:", on_mouse_moved as *const c_void, "v@:@");
    add(view, "mouseMoved:", on_mouse_moved as *const c_void, "v@:@");
    add(view, "rightMouseDown:", on_right_down as *const c_void, "v@:@");
    add(view, "rightMouseUp:", on_right_up as *const c_void, "v@:@");
    add(view, "rightMouseDragged:", on_mouse_moved as *const c_void, "v@:@");
    add(view, "otherMouseDown:", on_other_down as *const c_void, "v@:@");
    add(view, "otherMouseUp:", on_other_up as *const c_void, "v@:@");
    add(view, "otherMouseDragged:", on_mouse_moved as *const c_void, "v@:@");
    add(view, "mouseExited:", on_mouse_exited as *const c_void, "v@:@");
    add(view, "scrollWheel:", on_scroll as *const c_void, "v@:@");
    add(view, "magnifyWithEvent:", on_magnify as *const c_void, "v@:@");
    add(view, "keyDown:", on_key_down as *const c_void, "v@:@");
    add(view, "keyUp:", on_key_up as *const c_void, "v@:@");
    add(view, "flagsChanged:", on_flags_changed as *const c_void, "v@:@");
    add(view, "performKeyEquivalent:", on_key_equivalent as *const c_void, "c@:@");
    add(view, "viewDidChangeBackingProperties", on_backing_changed as *const c_void, "v@:");
    add(view, "draggingEntered:", dragging_entered as *const c_void, "Q@:@");
    add(view, "performDragOperation:", perform_drag as *const c_void, "c@:@");
    objc_registerClassPair(view);

    let wd = objc_allocateClassPair(cls("NSObject"), c"AlphaCadWindowDelegate".as_ptr(), 0);
    add(wd, "windowShouldClose:", window_should_close as *const c_void, "c@:@");
    add(wd, "windowDidResize:", window_did_resize as *const c_void, "v@:@");
    add(wd, "windowDidChangeBackingProperties:", window_did_resize as *const c_void, "v@:@");
    objc_registerClassPair(wd);

    let ad = objc_allocateClassPair(cls("NSObject"), c"AlphaCadAppDelegate".as_ptr(), 0);
    add(ad, "applicationShouldTerminate:", app_should_terminate as *const c_void, "Q@:@");
    add(ad, "application:openFiles:", app_open_files as *const c_void, "v@:@@");
    objc_registerClassPair(ad);
    (view, wd, ad)
}

unsafe fn build_menu(app: Id) {
    let menubar: Id = msg!(msg!(cls("NSMenu"), "alloc"; Id), "init"; Id);
    let app_item: Id = msg!(msg!(cls("NSMenuItem"), "alloc"; Id), "init"; Id);
    msg!(menubar, "addItem:", app_item => Id; ());
    let app_menu: Id = msg!(msg!(cls("NSMenu"), "alloc"; Id), "init"; Id);
    let ru = super::system_prefers_russian();
    let about = if ru { "О программе AlphaCad" } else { "About AlphaCad" };
    let hide = if ru { "Скрыть AlphaCad" } else { "Hide AlphaCad" };
    let quit = if ru { "Завершить AlphaCad" } else { "Quit AlphaCad" };
    msg!(app_menu, "addItemWithTitle:action:keyEquivalent:", nsstring(about) => Id, sel("orderFrontStandardAboutPanel:") => Sel, nsstring("") => Id; Id);
    let sep: Id = msg!(cls("NSMenuItem"), "separatorItem"; Id);
    msg!(app_menu, "addItem:", sep => Id; ());
    msg!(app_menu, "addItemWithTitle:action:keyEquivalent:", nsstring(hide) => Id, sel("hide:") => Sel, nsstring("h") => Id; Id);
    let sep2: Id = msg!(cls("NSMenuItem"), "separatorItem"; Id);
    msg!(app_menu, "addItem:", sep2 => Id; ());
    msg!(app_menu, "addItemWithTitle:action:keyEquivalent:", nsstring(quit) => Id, sel("terminate:") => Sel, nsstring("q") => Id; Id);
    msg!(app_item, "setSubmenu:", app_menu => Id; ());
    msg!(app, "setMainMenu:", menubar => Id; ());
}

/// Главный цикл окна.
pub fn run(handler: Box<dyn Handler>, title: &str, width: u32, height: u32) -> Result<(), String> {
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let app: Id = msg!(cls("NSApplication"), "sharedApplication"; Id);
        if app.is_null() {
            return Err("NSApplication недоступен".into());
        }
        msg!(app, "setActivationPolicy:", 0isize => isize; Bool);
        let (view_cls, wd_cls, ad_cls) = register_classes();
        let app_delegate: Id = msg!(msg!(ad_cls, "alloc"; Id), "init"; Id);
        msg!(app, "setDelegate:", app_delegate => Id; ());
        build_menu(app);
        APP.with(|a| *a.borrow_mut() = Some(handler));
        msg!(app, "finishLaunching"; ());

        // Окно по размеру экрана.
        let screen: Id = msg!(cls("NSScreen"), "mainScreen"; Id);
        let vis = if screen.is_null() { NSRect { origin: NSPoint::default(), size: NSSize { w: 1440.0, h: 900.0 } } } else { msg_rect(screen, "visibleFrame") };
        let w = (width as f64).min(vis.size.w * 0.92);
        let h = (height as f64).min(vis.size.h * 0.92);
        let rect = NSRect { origin: NSPoint { x: 0.0, y: 0.0 }, size: NSSize { w, h } };
        let style: u64 = 1 | 2 | 4 | 8; // titled | closable | miniaturizable | resizable
        let window: Id = msg!(msg!(cls("NSWindow"), "alloc"; Id), "initWithContentRect:styleMask:backing:defer:", rect => NSRect, style => u64, 2u64 => u64, NO => Bool; Id);
        if window.is_null() {
            return Err("не удалось создать NSWindow".into());
        }
        WINDOW.with(|c| c.set(window));
        msg!(window, "setReleasedWhenClosed:", NO => Bool; ());
        msg!(window, "setTitle:", nsstring(title) => Id; ());
        msg!(window, "setContentMinSize:", NSSize { w: 720.0, h: 480.0 } => NSSize; ());
        msg!(window, "setAcceptsMouseMovedEvents:", YES => Bool; ());
        let delegate: Id = msg!(msg!(wd_cls, "alloc"; Id), "init"; Id);
        msg!(window, "setDelegate:", delegate => Id; ());

        let view: Id = msg!(msg!(view_cls, "alloc"; Id), "initWithFrame:", rect => NSRect; Id);
        VIEW.with(|c| c.set(view));
        msg!(window, "setContentView:", view => Id; ());
        msg!(window, "makeFirstResponder:", view => Id; Bool);
        // Отслеживание выхода мыши за пределы вида.
        let opts: u64 = 0x01 | 0x02 | 0x80 | 0x200; // EnteredAndExited | MouseMoved | ActiveAlways | InVisibleRect
        let ta: Id = msg!(msg!(cls("NSTrackingArea"), "alloc"; Id), "initWithRect:options:owner:userInfo:", NSRect::default() => NSRect, opts => u64, view => Id, null_mut() => Id; Id);
        msg!(view, "addTrackingArea:", ta => Id; ());
        let types = nsarray(&[nsstring("public.file-url")]);
        msg!(view, "registerForDraggedTypes:", types => Id; ());

        msg!(window, "center"; ());
        send_resize();
        pump_frame();
        msg!(window, "makeKeyAndOrderFront:", null_mut() => Id; ());
        msg!(app, "activateIgnoringOtherApps:", YES => Bool; ());
        objc_autoreleasePoolPop(pool);

        let mode = NSDefaultRunLoopMode;
        loop {
            let pool = objc_autoreleasePoolPush();
            let dirty = DIRTY.with(|d| d.get()) || PENDING_FILES.with(|p| !p.borrow().is_empty());
            let until: Id = if dirty {
                msg!(cls("NSDate"), "distantPast"; Id)
            } else if let Some(t) = WAKE.with(|w| w.get()) {
                let secs = t.saturating_duration_since(Instant::now()).as_secs_f64();
                msg!(cls("NSDate"), "dateWithTimeIntervalSinceNow:", secs => f64; Id)
            } else {
                msg!(cls("NSDate"), "distantFuture"; Id)
            };
            let mut ev: Id = msg!(app, "nextEventMatchingMask:untilDate:inMode:dequeue:", u64::MAX => u64, until => Id, mode => Id, YES => Bool; Id);
            while !ev.is_null() {
                msg!(app, "sendEvent:", ev => Id; ());
                let past: Id = msg!(cls("NSDate"), "distantPast"; Id);
                ev = msg!(app, "nextEventMatchingMask:untilDate:inMode:dequeue:", u64::MAX => u64, past => Id, mode => Id, YES => Bool; Id);
            }
            msg!(app, "updateWindows"; ());
            let wake_due = WAKE.with(|w| w.get()).map(|t| Instant::now() >= t).unwrap_or(false);
            if DIRTY.with(|d| d.get()) || wake_due || PENDING_FILES.with(|p| !p.borrow().is_empty()) {
                if wake_due {
                    WAKE.with(|w| w.set(None));
                }
                pump_frame();
            }
            objc_autoreleasePoolPop(pool);
            if QUIT.with(|q| q.get()) {
                break;
            }
        }
        msg!(window, "orderOut:", null_mut() => Id; ());
        APP.with(|a| *a.borrow_mut() = None);
    }
    Ok(())
}

fn all_extensions(filters: &[Filter]) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for (_, exts) in filters {
        for e in exts.iter() {
            if !v.iter().any(|x| x == e) {
                v.push(e.to_string());
            }
        }
    }
    v
}

pub fn open_file_dialog(title: &str, filters: &[Filter], multiple: bool) -> Vec<PathBuf> {
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let panel: Id = msg!(cls("NSOpenPanel"), "openPanel"; Id);
        msg!(panel, "setMessage:", nsstring(title) => Id; ());
        msg!(panel, "setCanChooseFiles:", YES => Bool; ());
        msg!(panel, "setCanChooseDirectories:", NO => Bool; ());
        let multi: Bool = if multiple { YES } else { NO };
        msg!(panel, "setAllowsMultipleSelection:", multi => Bool; ());
        let exts: Vec<Id> = all_extensions(filters).iter().map(|e| nsstring(e)).collect();
        if !exts.is_empty() {
            msg!(panel, "setAllowedFileTypes:", nsarray(&exts) => Id; ());
        }
        let r: isize = msg!(panel, "runModal"; isize);
        let mut out = Vec::new();
        if r == 1 {
            let urls: Id = msg!(panel, "URLs"; Id);
            for u in array_items(urls) {
                let p = rust_string(msg!(u, "path"; Id));
                if !p.is_empty() {
                    out.push(PathBuf::from(p));
                }
            }
        }
        restore_focus();
        objc_autoreleasePoolPop(pool);
        out
    }
}

pub fn save_file_dialog(title: &str, default_name: &str, filters: &[Filter]) -> Option<PathBuf> {
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let panel: Id = msg!(cls("NSSavePanel"), "savePanel"; Id);
        msg!(panel, "setMessage:", nsstring(title) => Id; ());
        msg!(panel, "setNameFieldStringValue:", nsstring(default_name) => Id; ());
        let exts: Vec<Id> = all_extensions(filters).iter().map(|e| nsstring(e)).collect();
        if !exts.is_empty() {
            msg!(panel, "setAllowedFileTypes:", nsarray(&exts) => Id; ());
        }
        msg!(panel, "setCanCreateDirectories:", YES => Bool; ());
        let r: isize = msg!(panel, "runModal"; isize);
        let mut out = None;
        if r == 1 {
            let url: Id = msg!(panel, "URL"; Id);
            if !url.is_null() {
                let p = rust_string(msg!(url, "path"; Id));
                if !p.is_empty() {
                    out = Some(PathBuf::from(p));
                }
            }
        }
        restore_focus();
        objc_autoreleasePoolPop(pool);
        out
    }
}

unsafe fn restore_focus() {
    let w = WINDOW.with(|w| w.get());
    if !w.is_null() {
        msg!(w, "makeKeyAndOrderFront:", null_mut() => Id; ());
    }
    // Кнопки мыши могли быть отпущены внутри диалога.
    push(Event::MouseLeave);
}

pub fn message_box(title: &str, text: &str) {
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let app: Id = msg!(cls("NSApplication"), "sharedApplication"; Id);
        msg!(app, "setActivationPolicy:", 0isize => isize; Bool);
        msg!(app, "activateIgnoringOtherApps:", YES => Bool; ());
        let alert: Id = msg!(msg!(cls("NSAlert"), "alloc"; Id), "init"; Id);
        msg!(alert, "setMessageText:", nsstring(title) => Id; ());
        msg!(alert, "setInformativeText:", nsstring(text) => Id; ());
        msg!(alert, "runModal"; isize);
        objc_autoreleasePoolPop(pool);
    }
}
