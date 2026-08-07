# 本地安装包归档

这个目录用于在项目工作区内统一保存 macOS 和 Windows 安装包。实际二进制位于按版本划分的 `v*` 子目录中，并由项目 `.gitignore` 排除，避免大型安装包进入 Git 历史。

推荐目录结构：

```text
release-assets/
├── README.md
└── v0.1.7/
    ├── CHANGELOG.md
    ├── README.zh-CN.md
    ├── SHA256SUMS.txt
    ├── macOS/
    │   └── Codex Status Floater_0.1.7_aarch64_adhoc.dmg
    └── Windows/
        ├── Codex Status Floater_0.1.7_x64-setup.exe
        └── Codex Status Floater_0.1.7_x64_en-US.msi
```

每次发布时：

1. 在 `release-assets/` 下创建对应版本目录。
2. 放入 macOS、Windows 安装包、中文说明、更新日志和 SHA-256 校验文件。
3. 校验本地文件后，将同一批文件上传到对应的 GitHub Release。
4. 不要使用 `git add -f` 强制提交版本目录中的安装包。

GitHub 发布页：[0x0BB9/codex-status Releases](https://github.com/0x0BB9/codex-status/releases)
