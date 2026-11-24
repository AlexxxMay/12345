//! Сборочный скрипт: для Windows (MSVC) формирует файл ресурсов `.res`
//! с иконкой приложения, манифестом (Per-Monitor DPI V2, Common Controls 6)
//! и сведениями о версии — без внешних инструментов (rc.exe не нужен).

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon/alphacad.ico");
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || target_env != "msvc" {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let ico = fs::read("assets/icon/alphacad.ico").expect("assets/icon/alphacad.ico");
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.1.0".into());
    let res = build_res(&ico, &manifest(), &version);
    let path = out.join("alphacad.res");
    fs::write(&path, res).unwrap();
    println!("cargo:rustc-link-arg-bins={}", path.display());
    println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
}

fn manifest() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="AlphaCad" version="1.0.0.0" processorArchitecture="amd64"/>
  <description>AlphaCad</description>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2, PerMonitor</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
      <supportedOS Id="{1f676c76-80e1-4239-95bb-83d0f6d0da78}"/>
      <supportedOS Id="{4a2f28e3-53b9-4441-ba9c-d69d4a4a6e38}"/>
      <supportedOS Id="{35138b9a-5d96-4fbd-8e2d-a2440225f93a}"/>
    </application>
  </compatibility>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#
    .to_string()
}

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn u32le(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

/// Запись одного ресурса с числовыми типом и именем.
fn resource(out: &mut Vec<u8>, ty: u16, name: u16, flags: u16, lang: u16, data: &[u8]) {
    u32le(out, data.len() as u32);
    u32le(out, 32);
    u16le(out, 0xFFFF);
    u16le(out, ty);
    u16le(out, 0xFFFF);
    u16le(out, name);
    u32le(out, 0); // DataVersion
    u16le(out, flags);
    u16le(out, lang);
    u32le(out, 0); // Version
    u32le(out, 0); // Characteristics
    out.extend_from_slice(data);
    pad4(out);
}

fn build_res(ico: &[u8], manifest: &str, version: &str) -> Vec<u8> {
    let mut out = Vec::new();
    // Пустой первый ресурс — сигнатура 32-битного .res.
    resource(&mut out, 0, 0, 0, 0, &[]);
    let lang = 0x0409;
    let count = u16::from_le_bytes([ico[4], ico[5]]) as usize;
    let mut group = Vec::new();
    u16le(&mut group, 0);
    u16le(&mut group, 1);
    u16le(&mut group, count as u16);
    for i in 0..count {
        let e = &ico[6 + i * 16..6 + (i + 1) * 16];
        let size = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let off = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        let id = (i + 1) as u16;
        resource(&mut out, 3, id, 0x1010, lang, &ico[off..off + size]);
        group.extend_from_slice(&e[0..12]);
        u16le(&mut group, id);
    }
    resource(&mut out, 14, 1, 0x1030, lang, &group);
    resource(&mut out, 24, 1, 0x0030, lang, manifest.as_bytes());
    resource(&mut out, 16, 1, 0x0030, lang, &version_info(version));
    out
}

// --- VS_VERSIONINFO --------------------------------------------------------

fn utf16z(s: &str) -> Vec<u8> {
    let mut v = Vec::new();
    for c in s.encode_utf16() {
        u16le(&mut v, c);
    }
    u16le(&mut v, 0);
    v
}

/// Узел версии: wLength, wValueLength, wType, szKey, padding, Value, padding, Children.
fn node(key: &str, value: &[u8], value_len: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut v = Vec::new();
    u16le(&mut v, 0); // длина — позже
    u16le(&mut v, value_len);
    u16le(&mut v, if text { 1 } else { 0 });
    v.extend_from_slice(&utf16z(key));
    pad4(&mut v);
    v.extend_from_slice(value);
    for c in children {
        pad4(&mut v);
        v.extend_from_slice(c);
    }
    let len = v.len() as u16;
    v[0..2].copy_from_slice(&len.to_le_bytes());
    v
}

fn string_node(key: &str, val: &str) -> Vec<u8> {
    let data = utf16z(val);
    node(key, &data, (data.len() / 2) as u16, true, &[])
}

fn version_info(version: &str) -> Vec<u8> {
    let nums: Vec<u16> = version.split('.').map(|p| p.parse().unwrap_or(0)).chain(std::iter::repeat(0)).take(4).collect();
    let ms = ((nums[0] as u32) << 16) | nums[1] as u32;
    let ls = ((nums[2] as u32) << 16) | nums[3] as u32;
    let mut fixed = Vec::new();
    u32le(&mut fixed, 0xFEEF04BD);
    u32le(&mut fixed, 0x0001_0000);
    u32le(&mut fixed, ms);
    u32le(&mut fixed, ls);
    u32le(&mut fixed, ms);
    u32le(&mut fixed, ls);
    u32le(&mut fixed, 0x3F);
    u32le(&mut fixed, 0);
    u32le(&mut fixed, 0x0004_0004); // VOS_NT_WINDOWS32
    u32le(&mut fixed, 1); // VFT_APP
    u32le(&mut fixed, 0);
    u32le(&mut fixed, 0);
    u32le(&mut fixed, 0);
    let strings = vec![
        string_node("CompanyName", "AlphaCad"),
        string_node("FileDescription", "AlphaCad — 3D CAD editor"),
        string_node("FileVersion", version),
        string_node("InternalName", "alphacad"),
        string_node("LegalCopyright", "MIT License"),
        string_node("OriginalFilename", "alphacad.exe"),
        string_node("ProductName", "AlphaCad"),
        string_node("ProductVersion", version),
    ];
    let table = node("040904B0", &[], 0, true, &strings);
    let sfi = node("StringFileInfo", &[], 0, true, &[table]);
    let mut trans = Vec::new();
    u16le(&mut trans, 0x0409);
    u16le(&mut trans, 0x04B0);
    let var = node("Translation", &trans, 4, false, &[]);
    let vfi = node("VarFileInfo", &[], 0, true, &[var]);
    node("VS_VERSION_INFO", &fixed, fixed.len() as u16, false, &[sfi, vfi])
}
