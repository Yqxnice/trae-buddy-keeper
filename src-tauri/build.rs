fn main() {
    let attributes = tauri_build::Attributes::new();
    // 关闭 tauri-build 的默认应用清单：它只被链进 bins（主二进制），
    // 而单元测试 target 的 kind 仍是 lib，拿不到这份清单。
    #[cfg(windows)]
    let attributes =
        attributes.windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());

    tauri_build::try_build(attributes).expect("failed to run tauri-build");

    // 改用链接参数为所有产物（主二进制 / cdylib / 单元测试 / 集成测试）嵌入同一份清单。
    // 否则 Windows 上单元测试 exe 会因缺少 Microsoft.Windows.Common-Controls v6，
    // 在加载期直接以 STATUS_ENTRYPOINT_NOT_FOUND (0xc0000139) 崩溃，测试根本跑不起来。
    // 清单内容与 tauri-build 默认模板一致，生产二进制行为不变。
    #[cfg(windows)]
    {
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
}
