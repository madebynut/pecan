# Pecan · 碧果

黑白灰坚果轮廓，保留碧根果的纵向纹理；深灰底板和白色主体，小尺寸保持清晰。

- `pecan-icon.svg`：可编辑矢量源图。
- `pecan-icon-256.png`：应用内部的品牌图标。
- `pecan.ico`：Windows 多尺寸图标（16–256px），嵌入 EXE。
- `pecan.icns`：macOS 多尺寸图标，含 Retina 尺寸。

执行 `sh scripts/build-icons.sh`，使用现有 resvg 依赖生成 PNG，再写入标准 macOS ICNS 容器。运行时通过 OnceLock 复用图像。
