import worker from "../../crates/veil-decoy-edge/worker.js";

export default {
    ...worker,
    async fetch(request, env, ctx) {
        if (new URL(request.url).pathname !== "/_lab/retry") {
            return worker.fetch(request, env, ctx);
        }
        if (request.method !== "POST" ||
            request.headers.get("Authorization") !== "Bearer " + env.VEIL_ADMIN_TOKEN) {
            return new Response(null, { status: 401 });
        }
        const work = [];
        worker.scheduled({}, env, { waitUntil: (task) => work.push(task) });
        await Promise.all(work);
        return new Response(null, { status: 204 });
    },
};
