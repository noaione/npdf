use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=src/error_shim.c");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // Adjust path to point to third_party/libjxl from simple-jpegli-enc/
    let libjxl_src = manifest_dir.join("../third_party/libjxl");

    cc::Build::new()
        .file("src/error_shim.c")
        .include(&manifest_dir)
        .compile("jpegli_error_shim");

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

    println!("cargo:rustc-link-lib=static=jpegli-static");
    println!("cargo:rustc-link-lib=static=hwy");

    // Generate bindings
    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .header(format!(
            "{}/build/lib/include/jpegli/jpeglib.h",
            dst.display()
        ))
        .clang_arg(format!("-I{}/build/lib/include/jpegli", dst.display()))
        // We need to include the system headers or at least the standard types
        .use_core()
        .ctypes_prefix("libc")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
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
