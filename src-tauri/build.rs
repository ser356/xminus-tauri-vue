use std::{env, fs, path::Path};

// Clave del ofuscado XOR. No es cifrado: solo evita que la cookie aparezca
// en texto plano al inspeccionar el binario con `strings`.
const KEY: &[u8] = b"xminus-stems-embedded";

/// Lee un secreto opcional de una variable de entorno o de `secrets/<file>`.
fn secret(var: &str, file: &str, strip_prefix: &str) -> Option<String> {
    println!("cargo:rerun-if-env-changed={var}");
    let raw = env::var(var).ok()
        .or_else(|| fs::read_to_string(Path::new("secrets").join(file)).ok())?;
    let mut value = raw.trim();
    if value.len() >= strip_prefix.len()
        && value[..strip_prefix.len()].eq_ignore_ascii_case(strip_prefix) {
        value = value[strip_prefix.len()..].trim();
    }
    if value.is_empty() {
        return None;
    }
    if value.contains(['\r', '\n']) {
        panic!("secrets/{file}: el valor debe estar en una sola línea");
    }
    Some(value.to_owned())
}

fn obfuscate(value: &str) -> String {
    value.bytes().enumerate()
        .map(|(i, b)| format!("{:02x}", b ^ KEY[i % KEY.len()]))
        .collect()
}

fn main() {
    // Escanea la carpeta completa: detecta cuándo se añade o edita cookie.txt.
    println!("cargo:rerun-if-changed=secrets");
    if let Some(cookie) = secret("XMINUS_COOKIE", "cookie.txt", "cookie:") {
        println!("cargo:rustc-env=XMINUS_EMBEDDED_COOKIE={}", obfuscate(&cookie));
    }
    // Opcional: algunas cookies (p. ej. cf_clearance) solo valen con el
    // mismo User-Agent del navegador del que se copiaron.
    if let Some(agent) = secret("XMINUS_USER_AGENT", "user_agent.txt", "user-agent:") {
        println!("cargo:rustc-env=XMINUS_EMBEDDED_USER_AGENT={agent}");
    }
    tauri_build::build()
}
