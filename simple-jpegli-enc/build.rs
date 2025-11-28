use std::env;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sanitizer {
    Address,
    Leak,
    Thread,
    Undefined,
}

impl Sanitizer {
    fn detect(target: &str) -> Result<Option<Self>, String> {
        let mut enabled = Vec::new();

        if env::var_os("CARGO_FEATURE_ASAN").is_some() {
            enabled.push(Sanitizer::Address);
        }
        if env::var_os("CARGO_FEATURE_LSAN").is_some() {
            enabled.push(Sanitizer::Leak);
        }
        if env::var_os("CARGO_FEATURE_TSAN").is_some() {
            enabled.push(Sanitizer::Thread);
        }
        if env::var_os("CARGO_FEATURE_UBSAN").is_some() {
            enabled.push(Sanitizer::Undefined);
        }

        if enabled.len() > 1 {
            return Err("more than one sanitizer feature enabled; choose exactly one".to_string());
        }

        let sanitizer = enabled.into_iter().next();

        if let Some(choice) = sanitizer {
            if target.contains("windows-msvc") {
                return Err(format!(
                    "{} sanitizer is not supported on MSVC targets",
                    choice.display_name()
                ));
            }

            if target.contains("apple") && matches!(choice, Sanitizer::Leak) {
                return Err(format!(
                    "{} sanitizer is not available on Apple targets",
                    choice.display_name()
                ));
            }
        }

        Ok(sanitizer)
    }

    fn display_name(self) -> &'static str {
        match self {
            Sanitizer::Address => "AddressSanitizer",
            Sanitizer::Leak => "LeakSanitizer",
            Sanitizer::Thread => "ThreadSanitizer",
            Sanitizer::Undefined => "UndefinedBehaviorSanitizer",
        }
    }

    fn compile_flags(self) -> &'static [&'static str] {
        match self {
            Sanitizer::Address => &["-fsanitize=address", "-fno-omit-frame-pointer"],
            Sanitizer::Leak => &["-fsanitize=leak", "-fno-omit-frame-pointer"],
            Sanitizer::Thread => &["-fsanitize=thread"],
            Sanitizer::Undefined => &["-fsanitize=undefined"],
        }
    }

    fn link_args(self) -> &'static [&'static str] {
        match self {
            Sanitizer::Address => &["-fsanitize=address"],
            Sanitizer::Leak => &["-fsanitize=leak"],
            Sanitizer::Thread => &["-fsanitize=thread"],
            Sanitizer::Undefined => &["-fsanitize=undefined"],
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=ffi/bridge.h");
    println!("cargo:rerun-if-changed=ffi/bridge.cc");

    let target = env::var("TARGET").expect("TARGET not set");
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // Adjust path to point to third_party/libjxl from simple-jpegli-enc/
    let libjxl_src = manifest_dir.join("../third_party/libjxl");

    let sanitizer = Sanitizer::detect(&target)
        .unwrap_or_else(|err| panic!("sanitizer configuration error: {err}"));

    if let Some(selected) = sanitizer {
        println!(
            "cargo:warning=Enabling {} instrumentation",
            selected.display_name()
        );
    }

    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let cmake_build_type = if profile == "debug" {
        "Debug"
    } else {
        "Release"
    };

    // Detect whether Rust itself is requesting a static CRT (Windows MSVC only)
    let wants_static_crt = std::env::var("CARGO_CFG_TARGET_FEATURE")
        .unwrap_or_default()
        .split(',')
        .any(|f| f == "crt-static");

    let mut cfg = cmake::Config::new(&libjxl_src);
    cfg.profile(cmake_build_type)
        .static_crt(wants_static_crt)
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
        .define("JPEGXL_FORCE_SYSTEM_HWY", "OFF")
        .define("JPEGXL_ENABLE_HWY_AVX3", "true")
        .define("JPEGXL_ENABLE_HWY_AVX3_SPR", "true")
        .define("JPEGXL_ENABLE_HWY_AVX3_ZEN4", "true")
        .define("JPEGXL_ENABLE_HWY_AVX3_SSSE3", "true")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("BUILD_TESTING", "OFF")
        .define("JPEGLI_LIBJPEG_LIBRARY_SOVERSION", "8")
        .define("JPEGLI_LIBJPEG_LIBRARY_VERSION", "8.2.2")
        .build_target("jpegli-static");

    if cmake_build_type == "Debug" && cfg!(target_os = "windows") {
        // On Windows debug builds, ensure we link against the release runtime to avoid
        // issues with mixing debug/release CRTs.
        cfg.define("CMAKE_MSVC_RUNTIME_LIBRARY", "MultiThreadedDLL");
    }

    if let Some(s) = sanitizer {
        apply_sanitizer_to_cmake(&mut cfg, s);
    }

    let dst = cfg.build();

    let build_root = dst.join("build");
    let lib_dir = build_root.join("lib");
    let hwy_dir = build_root.join("third_party/highway");

    println!("cargo:rustc-link-search=native={}/build/lib", dst.display());
    if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:rustc-link-search=native={}", hwy_dir.display());
    } else {
        // Windows, use nested path
        println!(
            "cargo:rustc-link-search=native={}",
            lib_dir.join(cmake_build_type).display()
        );
        // or fallback to the same mac/linux search path
        println!("cargo:rustc-link-search=native={}", lib_dir.display());

        // Use nested path for highway as well
        println!(
            "cargo:rustc-link-search=native={}",
            hwy_dir.join(cmake_build_type).display()
        );
        // fallback again
        println!("cargo:rustc-link-search=native={}", hwy_dir.display());
    }

    emit_system_library_hints();

    // Always link static form explicitly for consistency.
    println!("cargo:rustc-link-lib=static=jpegli-static");
    println!("cargo:rustc-link-lib=static=hwy");

    compile_bridge(&manifest_dir, &libjxl_src, &dst, sanitizer);
}

fn compile_bridge(
    manifest_dir: &Path,
    libjxl_src: &Path,
    libjxl_dst: &Path,
    sanitizer: Option<Sanitizer>,
) {
    // Detect whether Rust itself is requesting a static CRT (Windows MSVC only)
    let wants_static_crt = std::env::var("CARGO_CFG_TARGET_FEATURE")
        .unwrap_or_default()
        .split(',')
        .any(|f| f == "crt-static");

    let mut build = cc::Build::new();

    build
        .cpp(true)
        .file(manifest_dir.join("ffi/bridge.cc"))
        .include(manifest_dir.join("ffi"))
        .include(libjxl_dst.join("build/lib/include/jpegli"))
        .include(libjxl_src)
        .define("WIN32_LEAN_AND_MEAN", None)
        .define("NOMINMAX", None)
        .flag_if_supported("-std=c++20")
        .flag_if_supported("/std:c++20")
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-variable");

    // Force static or dynamic CRT linkage based on Rust's own settings
    // Ignore any debug mode build
    if wants_static_crt {
        build.flag_if_supported("/MT");
    } else {
        build.flag_if_supported("/MD");
    }

    if let Some(s) = sanitizer {
        apply_sanitizer_to_cc(&mut build, s);
    }

    build.compile("simple_jpegli_enc_bridge");
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

fn apply_sanitizer_to_cmake(cfg: &mut cmake::Config, sanitizer: Sanitizer) {
    let compile_flags = sanitizer.compile_flags();
    for flag in compile_flags {
        cfg.cflag(flag);
        cfg.cxxflag(flag);
    }

    let link_flags = sanitizer.link_args();
    if !link_flags.is_empty() {
        let joined = link_flags.join(" ");
        cfg.define("CMAKE_EXE_LINKER_FLAGS", &joined);
        cfg.define("CMAKE_SHARED_LINKER_FLAGS", &joined);
        cfg.define("CMAKE_MODULE_LINKER_FLAGS", &joined);
    }
}

fn apply_sanitizer_to_cc(build: &mut cc::Build, sanitizer: Sanitizer) {
    for flag in sanitizer.compile_flags() {
        build.flag(flag);
    }
}
