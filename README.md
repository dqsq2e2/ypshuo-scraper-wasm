# 阅评说刮削插件 (WASM)

Ting Reader 的阅评说 (ypshuo.com) 刮削插件，基于 WASM 技术实现。

## 编译

本插件需要 `wasm32-wasip1` 目标支持：

```bash
# 添加构建目标
rustup target add wasm32-wasip1

# 编译
cargo build --target wasm32-wasip1 --release
```

编译产物位于：`target/wasm32-wasip1/release/ypshuo_scraper.wasm`

## 安装

1. 将 `ypshuo_scraper.wasm` 复制到插件目录
2. 将 `plugin.json` 复制到同一目录
3. 重启 Ting Reader 即可自动加载

## 说明

- 插件通过 WASI 接口与宿主环境交互
- 使用 HTTP 请求获取阅评说网站数据
