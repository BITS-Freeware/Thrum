/// One-time initializer for the wasm core module. The generated loader
/// fetches `geist3_wasm_bg.wasm` relative to its own module URL, which Vite
/// serves as a static asset in both dev and build.

import init from "./wasm/geist3_wasm.js";

let ready: Promise<unknown> | null = null;

/** Load the wasm module once; every caller awaits the same instance. */
export function initWasm(): Promise<unknown> {
  if (!ready) ready = init();
  return ready;
}
