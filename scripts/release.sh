#!/usr/bin/env bash
# 在项目根目录完成构建后，本地生成桌面包、脚本包和校验清单。
# 不修改版本、不创建提交、不推送、不发布到远端。
set -euo pipefail
pnpm release:pack
