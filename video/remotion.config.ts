/**
 * Note: When using the Node.JS APIs, the config file
 * doesn't apply. Instead, pass options directly to the APIs.
 *
 * All configuration options: https://remotion.dev/docs/config
 */

import path from "node:path";
import { Config } from "@remotion/cli/config";

Config.setRspack(true);
Config.setVideoImageFormat("jpeg");
Config.setOverwriteOutput(true);
// The faces, icons and pigments are imported from ../ui/src: resolve their React from here.
// overrideBundlerConfig, not overrideWebpackConfig: with Rspack on, the webpack override is silently ignored.
Config.overrideBundlerConfig((c) => ({
  ...c,
  resolve: { ...c.resolve, modules: [path.resolve("node_modules"), "node_modules"] },
}));
