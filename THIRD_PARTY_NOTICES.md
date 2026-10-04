# Third-party notices

OCS Rust is an independently maintained derivative. Original ownership and license terms remain in effect.

| Component | Origin / baseline | License / notice |
| --- | --- | --- |
| OCS Desktop | [ocsjs/ocs-desktop](https://github.com/ocsjs/ocs-desktop/tree/ecc6bb7ee79cb713caab7e896a913383e9437a14), 2.12.0, enncy and contributors | MIT as declared in upstream package metadata; original authorship retained |
| OCS userscript | [ocsjs/ocsjs](https://github.com/ocsjs/ocsjs), script baseline 4.15.3, enncy | [Original MIT text](assets/licenses/OCS-userscript.LICENSE); script metadata retained |
| Rust integration | mohui666 and OCS Rust contributors | [MIT](LICENSE) |
| Node.js runtime | Node.js version recorded in the release manifest | License copied beside the bundled executable as `adapter/Node-LICENSE`; vendored license under `assets/licenses` |
| Playwright | Version locked by pnpm | Apache-2.0; LICENSE and NOTICE retained under `adapter/node_modules/playwright-core` |
| JavaScript dependencies | Versions in `pnpm-lock.yaml` | Source notices retained; esbuild legal comments emitted beside the adapter bundle |
| Rust dependencies | Versions in `Cargo.lock` | Their respective crate licenses; dependency source is obtained by Cargo |
| Course-platform icons | [Asset sources](packages/web/public/site-icons/SOURCES.md) | Brand assets belong to their respective owners; used for identifying navigation links, not relicensed by the project MIT license |

The desktop archive includes the relevant bundled runtime notices. The userscript archive includes the project license and original OCS userscript license. Repository MIT licensing does not grant rights to third-party trademarks or service accounts.
