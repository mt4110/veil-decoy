import { fetch as handle, initSync, retry } from "./build/veil_decoy_edge.js";
import module from "./build/veil_decoy_edge_bg.wasm";

initSync({ module });

export default {
    fetch: handle,
    scheduled(_controller, env, ctx) {
        ctx.waitUntil(retry(env));
    },
};
