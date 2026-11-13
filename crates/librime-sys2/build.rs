use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    #[cfg(target_os = "windows")]
    build_windows();

    #[cfg(not(target_os = "windows"))]
    build_unix();
}

fn librime_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("librime")
}

/// rime-lua 仓库（librime 的官方 Lua 插件）。
const LUA_PLUGIN_REPO: &str = "https://github.com/hchunhui/librime-lua.git";

/// 确认 `librime/plugins/lua` 存在且带可用的 Lua 源码（`thirdparty/lua5.4/lua.h`）。
///
/// librime 的 `plugins/CMakeLists.txt` 会把 `plugins/` 下的每个子目录当成插件编进
/// 主库（`BUILD_MERGED_PLUGINS`），插件模块名随后进入 `RIME_SETUP_EXTRA_MODULES`，
/// `lua_translator@*` / `lua_filter@*` / `lua_processor@*` 才能被解析。librime 仓库
/// 本身不带 rime-lua（`plugins/` 只跟踪三个文件），所以默认构建必须自己把它拉下来，
/// 否则依赖 Lua 的方案（本项目内置的 wubi86 / wubi98 / pinyin_simp，以及万象等
/// 第三方方案）会静默失效。
///
/// rime-lua 的 Lua 源码在 `thirdparty` 分支上，且该分支以 tree 条目存放 lua5.3/5.4/5.5，
/// 普通 `git clone -b thirdparty --depth 1` 就能拿到完整源码，不需要 submodule 递归。
///
/// 拉取失败时**移除残留目录**：只留下没有 Lua 源码的插件目录会让 CMake 在
/// `plugins/lua/CMakeLists.txt` 里 FATAL_ERROR，直接打断构建；宁可退回「没有 Lua」
/// 的旧行为，也不要让整个构建失败。
fn ensure_lua_plugin(librime: &std::path::Path) {
    let plugins_dir = librime.join("plugins");
    let plugin_dir = plugins_dir.join("lua");
    let lua_header = plugin_dir.join("thirdparty").join("lua5.4").join("lua.h");

    if plugin_dir.join("CMakeLists.txt").exists() && lua_header.exists() {
        return;
    }

    println!(
        "cargo:warning=Provisioning rime-lua plugin into {}",
        plugin_dir.display()
    );

    let git_ok = |args: &[&str]| -> bool {
        match Command::new("git").args(args).status() {
            Ok(s) => s.success(),
            Err(e) => {
                println!("cargo:warning=git {:?} failed: {}", args, e);
                false
            }
        }
    };

    let mut ready = true;

    if !plugin_dir.join("CMakeLists.txt").exists() {
        if plugin_dir.exists() {
            std::fs::remove_dir_all(&plugin_dir).ok();
        }
        if let Some(parent) = plugin_dir.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        ready = git_ok(&[
            "clone",
            "--depth",
            "1",
            LUA_PLUGIN_REPO,
            plugin_dir.to_str().unwrap(),
        ]);
    }

    if ready && !lua_header.exists() {
        let thirdparty = plugin_dir.join("thirdparty");
        if thirdparty.exists() {
            std::fs::remove_dir_all(&thirdparty).ok();
        }
        ready = git_ok(&[
            "clone",
            "--depth",
            "1",
            "--branch",
            "thirdparty",
            LUA_PLUGIN_REPO,
            thirdparty.to_str().unwrap(),
        ]);
    }

    if ready && lua_header.exists() {
        println!("cargo:warning=rime-lua plugin ready at {}", plugin_dir.display());
        return;
    }

    println!(
        "cargo:warning=rime-lua plugin unavailable; Lua-based schema components will be missing"
    );
    // 半个插件目录比没有插件更糟：CMake 会 FATAL_ERROR。先清掉再退出。
    if plugin_dir.exists() {
        std::fs::remove_dir_all(&plugin_dir).ok();
    }
}

#[allow(dead_code)]
fn check_pkg_config() -> Option<PathBuf> {
    if pkg_config::find_library("rime").is_ok() {
        println!("cargo:warning=Found librime via pkg-config");
        println!("cargo:rerun-if-changed=build.rs");
        Some(PathBuf::new())
    } else {
        None
    }
}

#[allow(dead_code)]
fn cmake_build(librime: &std::path::Path) -> Option<PathBuf> {
    if !librime.join("CMakeLists.txt").exists() {
        return None;
    }

    println!(
        "cargo:warning=Building librime from submodule at {}",
        librime.display()
    );

    let dist_dir = librime.join("dist");
    if dist_dir.join("lib").join("librime.so").exists()
        || dist_dir.join("lib").join("librime.dylib").exists()
    {
        println!(
            "cargo:warning=Using cached librime build in {}",
            dist_dir.display()
        );
        return Some(dist_dir);
    }

    let build_dir = librime.join("build");
    let install_prefix = format!("-DCMAKE_INSTALL_PREFIX={}", dist_dir.display());

    ensure_lua_plugin(librime);

    let status = Command::new("cmake")
        .args([
            "-B",
            build_dir.to_str().unwrap(),
            "-S",
            librime.to_str().unwrap(),
            "-DCMAKE_BUILD_TYPE=Release",
            "-DBUILD_SHARED_LIBS=ON",
            "-DBUILD_TEST=OFF",
            &install_prefix,
            "-DENABLE_EXTERNAL_PLUGINS=OFF",
            "-DENABLE_LOGGING=OFF",
        ])
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(_) => {
            println!("cargo:warning=cmake configure failed; install build deps:");
            println!("cargo:warning=  sudo apt install cmake build-essential libgoogle-glog-dev libyaml-cpp-dev");
            println!(
                "cargo:warning=  sudo dnf install cmake gcc-c++ google-glog-devel yaml-cpp-devel"
            );
            return None;
        }
        Err(e) => {
            println!(
                "cargo:warning=cmake not found: {}. Install cmake or system librime.",
                e
            );
            return None;
        }
    }

    let nproc = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    let status = Command::new("cmake")
        .args([
            "--build",
            build_dir.to_str().unwrap(),
            // VS 多配置生成器必须显式指定 Release：不传时默认 Debug，
            // 而 `build/` 里可能只有 Release 配置（`MSB8013` 就是这么来的）。
            "--config",
            "Release",
            "--target",
            "install",
            "-j",
            &nproc.to_string(),
        ])
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("cargo:warning=librime compilation finished successfully");
            Some(dist_dir)
        }
        Ok(_) => {
            println!("cargo:warning=cmake build failed; falling back to system librime hint");
            None
        }
        Err(e) => {
            println!("cargo:warning=cmake build failed: {}", e);
            None
        }
    }
}

#[allow(dead_code)]
fn link_librime(dist: Option<PathBuf>) {
    if let Some(dist_dir) = dist {
        if dist_dir.as_os_str().is_empty() {
            return;
        }
        let lib = dist_dir.join("lib");
        println!("cargo:rustc-link-search=native={}", lib.display());
        println!("cargo:rustc-link-lib=dylib=rime");
        // 运行时优先加载子模块构建的 librime（如 dist 不存在则该条目回退系统库）；
        // 同时带上 $ORIGIN/../share/xime/lib——语音运行库（sherpa-onnx）安装位，
        // dev（~/.local/bin）与系统包（/usr/bin）布局通用。单条 -rpath 冒号
        // 分隔多路径：lld 下多条 -rpath 会互相覆盖。
        let rpath = lib.canonicalize().unwrap_or_else(|_| lib.clone());
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}:$ORIGIN/../share/xime/lib",
            rpath.to_string_lossy()
        );
    } else {
        println!("cargo:rustc-link-lib=dylib=rime");
        if let Ok(lib_dir) = env::var("RIME_LIB_DIR") {
            println!("cargo:rustc-link-search=native={}", lib_dir);
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}

// ── Windows ──────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn build_windows() {
    let librime = librime_dir();
    let dist_dir = librime.join("dist");
    let dist_lib_dir = dist_dir.join("lib");
    let rime_dll = dist_lib_dir.join("rime.dll");

    if !rime_dll.exists() {
        build_librime_source_win(&librime, &dist_lib_dir);
    }

    if rime_dll.exists() {
        println!("cargo:rustc-link-search=native={}", dist_lib_dir.display());
        println!("cargo:rustc-link-lib=dylib=rime");
        // rime.dll 要放在 exe 旁边。**不能**再用 `librime.parent()` 猜工作区：
        // 那是 libximecore 的目录，产物会落到 `libximecore/target/`，而真正要打包的
        // `XimeYao/target/<profile>/` 一直是旧文件（结果就是"重新编了 rime.dll 但
        // 实际跑的还是老的"）。用 cargo 给的 OUT_DIR 反推当前构建目录才可靠：
        //   <target>/<profile>/build/<pkg>-<hash>/out → 上溯 3 级是 <target>/<profile>
        let copied = std::env::var_os("OUT_DIR")
            .map(std::path::PathBuf::from)
            .and_then(|out| out.ancestors().nth(3).map(|p| p.to_path_buf()))
            .map(|profile_dir| {
                let dest = profile_dir.join("rime.dll");
                let ok = std::fs::copy(&rime_dll, &dest).is_ok();
                (dest, ok)
            });
        match copied {
            Some((dest, true)) => println!("cargo:warning=rime.dll → {}", dest.display()),
            Some((dest, false)) => {
                println!("cargo:warning=复制 rime.dll 到 {} 失败", dest.display())
            }
            None => println!("cargo:warning=拿不到 OUT_DIR，rime.dll 未复制到输出目录"),
        }
    } else {
        panic!(
            "librime build failed: rime.dll not found at {}",
            rime_dll.display()
        );
    }
}

#[cfg(target_os = "windows")]
fn build_librime_source_win(librime_dir: &std::path::Path, _dist_lib_dir: &std::path::Path) {
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    println!(
        "cargo:warning=Building librime from source (this may take 10+ minutes on first build)..."
    );

    fn find_vswhere() -> PathBuf {
        if let Ok(output) = Command::new("where").arg("vswhere").output() {
            if output.status.success() {
                let output = String::from_utf8_lossy(&output.stdout);
                for line in output.lines() {
                    let path = line.trim();
                    if !path.is_empty() {
                        let p = PathBuf::from(path);
                        if p.exists() {
                            return p;
                        }
                    }
                }
            }
        }
        for candidate in [
            r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe",
            r"C:\Program Files\Microsoft Visual Studio\Installer\vswhere.exe",
        ] {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return p;
            }
        }
        panic!("vswhere not found. Install Visual Studio 2022.");
    }

    let workspace_dir = librime_dir.parent().unwrap();
    ensure_lua_plugin(librime_dir);
    let vswhere = find_vswhere();
    let vs_install: String = match Command::new(&vswhere)
        .args(["-latest", "-property", "installationPath"])
        .output()
    {
        Ok(output) => String::from_utf8_lossy(&output.stdout).trim().to_string(),
        Err(e) => panic!("vswhere failed: {}", e),
    };
    if vs_install.is_empty() || !std::path::Path::new(&vs_install).exists() {
        panic!(
            "vswhere returned invalid VS installation path: {:?}. Install Visual Studio 2022.",
            vs_install
        );
    }

    let temp_bat = workspace_dir.join("temp-build-librime.bat");
    {
        let mut file = std::fs::File::create(&temp_bat).unwrap();
        writeln!(file, "@echo off").unwrap();
        writeln!(
            file,
            "call \"{}\\VC\\Auxiliary\\Build\\vcvars64.bat\"",
            vs_install
        )
        .unwrap();
        writeln!(file, "cd /d \"{}\"", librime_dir.display()).unwrap();
        writeln!(
            file,
            "if not exist \"{0}\\deps\\boost-1.89.0\\boost\" call install-boost.bat",
            librime_dir.display()
        )
        .unwrap();
        writeln!(
            file,
            "if not defined BOOST_ROOT set BOOST_ROOT={}\\deps\\boost-1.89.0",
            librime_dir.display()
        )
        .unwrap();
        writeln!(file, "build.bat deps librime shared").unwrap();
    }

    // librime 的 build.bat 会优先使用 env.bat（若存在则跳过 env.bat.template）。
    // template 写死 VS2019 + Win32，与 CI 上的 VS2022 不兼容，这里生成适配版本。
    let env_bat = librime_dir.join("env.bat");
    {
        let mut file = std::fs::File::create(&env_bat).unwrap();
        writeln!(file, "set RIME_ROOT={}", librime_dir.display()).unwrap();
        writeln!(
            file,
            "set BOOST_ROOT={}\\deps\\boost-1.89.0",
            librime_dir.display()
        )
        .unwrap();
        writeln!(file, "set ARCH=x64").unwrap();
        writeln!(file, "set BJAM_TOOLSET=msvc-14.3").unwrap();
        writeln!(file, "set CMAKE_GENERATOR=\"Visual Studio 17 2022\"").unwrap();
        writeln!(file, "set PLATFORM_TOOLSET=v143").unwrap();
    }

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    let progress_thread = thread::spawn(move || {
        let mut count = 0;
        while running_clone.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(30));
            if running_clone.load(Ordering::Relaxed) {
                count += 1;
                println!(
                    "cargo:warning=librime build in progress... ({} minutes elapsed)",
                    count / 2
                );
            }
        }
    });

    let status = Command::new(&temp_bat)
        .current_dir(workspace_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    running.store(false, Ordering::Relaxed);
    progress_thread.join().ok();
    std::fs::remove_file(&temp_bat).ok();
    let _ = std::fs::remove_file(&std::path::PathBuf::from(
        librime_dir.parent().unwrap().join("env.bat"),
    ));

    match status {
        Ok(s) if s.success() => println!("cargo:warning=librime compilation finished successfully"),
        Ok(s) => panic!("librime build failed with exit code: {:?}", s.code()),
        Err(e) => panic!("librime build failed to execute: {}", e),
    }
}

// ── Unix (Linux / macOS) ──────────────────────────────────────────────────

#[cfg(not(target_os = "windows"))]
fn build_unix() {
    let dist = check_pkg_config();
    if dist.is_some() {
        return link_librime(dist);
    }

    let librime = librime_dir();
    let dist = cmake_build(&librime);
    link_librime(dist);
}
