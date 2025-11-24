use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=ffi/bridge.h");
    println!("cargo:rerun-if-changed=ffi/bridge.cc");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // Adjust path to point to third_party/libjxl from simple-jpegli-enc/
    let libjxl_src = manifest_dir.join("../third_party/libjxl");

    let dst = cmake::Config::new(libjxl_src)
        .define("JPEGXL_ENABLE_JPEGLI", "ON")
        .define("JPEGXL_ENABLE_JPEGLI_LIBJPEG", "ON")
        .define("JPEGXL_ENABLE_TOOLS", "OFF")
        .define("JPEGXL_ENABLE_DOXYGEN", "OFF")
        .define("JPEGXL_ENABLE_MANPAGES", "OFF")
        .define("JPEGXL_ENABLE_BENCHMARK", "OFF")
        .define("JPEGXL_ENABLE_EXAMPLES", "OFF")
        .define("JPEGXL_ENABLE_JNI", "OFF")
        .define("JPEGXL_ENABLE_VIEWERS", "OFF")
        .define("JPEGXL_ENABLE_PLUGINS", "OFF")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("BUILD_TESTING", "OFF")
        .define("JPEGLI_LIBJPEG_LIBRARY_SOVERSION", "8")
        .define("JPEGLI_LIBJPEG_LIBRARY_VERSION", "8.2.2")
        .build_target("jpegli-static")
        .build();

    println!("cargo:rustc-link-search=native={}/build/lib", dst.display());
    if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
        println!(
            "cargo:rustc-link-search=native={}/build/third_party/highway",
            dst.display()
        );
    }
    emit_system_library_hints();

    // print the data to show the content of the dest path
    for entry in std::fs::read_dir(format!("{}/build/lib", dst.display())).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        println!("cargo:warning=Found file in build/lib: {}", path.display());
    }

    println!("cargo:rustc-link-lib=static=jpegli-static");
    println!("cargo:rustc-link-lib=static=hwy");

    compile_bridge(&manifest_dir, &dst);
}

fn compile_bridge(manifest_dir: &PathBuf, libjxl_dst: &PathBuf) {
    cc::Build::new()
        .cpp(true)
        .file(manifest_dir.join("ffi/bridge.cc"))
        .include(manifest_dir.join("ffi"))
        .include(libjxl_dst.join("build/lib/include/jpegli"))
        .define("WIN32_LEAN_AND_MEAN", None)
        .flag_if_supported("-std=c++20")
        .flag_if_supported("/std:c++20")
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-variable")
        .compile("simple_jpegli_enc_bridge");
}

fn emit_system_library_hints() {
    let libraries = vec![("HIGHWAY_DIR", "highway")];

    let loonix_search = vec![
        "/usr/lib/x86_64-linux-gnu",
        "/usr/local/lib/x86_64-linux-gnu",
        "/usr/lib",
        "/usr/local/lib",
    ];

    if cfg!(target_os = "linux") {
        // Try searching in common Linux library path
        for path in loonix_search {
            println!("cargo:rustc-link-search=native={}", path);
        }
    }

    if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    } else if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=stdc++");
    }

    if cfg!(target_os = "windows") {
        if let Ok(root) = env::var("VCPKG_ROOT") {
            let triplet =
                env::var("VCPKG_DEFAULT_TRIPLET").unwrap_or("x64-windows-static".to_string());
            let lib_path = PathBuf::from(root)
                .join("installed")
                .join(triplet)
                .join("lib");
            println!("cargo:rustc-link-search=native={}", lib_path.display());
        }
    } else {
        for (env_var, formula) in libraries {
            if let Some(dir) = env::var_os(env_var).map(PathBuf::from) {
                let lib_dir = dir.join("lib");
                if lib_dir.exists() {
                    println!("cargo:rustc-link-search=native={}", lib_dir.display());
                    continue;
                }
            }

            emit_homebrew_search_hint(formula);
        }
    }
}

fn emit_homebrew_search_hint(formula: &str) {
    let candidates = [
        env::var_os("HOMEBREW_PREFIX").map(PathBuf::from),
        env::var_os("BREW_PREFIX").map(PathBuf::from),
        Some(PathBuf::from("/opt/homebrew")),
        Some(PathBuf::from("/usr/local")),
    ];

    for prefix in candidates.into_iter().flatten() {
        let candidate = prefix.join("opt").join(formula).join("lib");
        if candidate.exists() {
            println!("cargo:rustc-link-search=native={}", candidate.display());
            break;
        }
    }
}
