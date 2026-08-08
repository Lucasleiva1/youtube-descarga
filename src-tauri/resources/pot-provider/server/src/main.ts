import { SessionManager } from "./session_manager.ts";
import { strerror, VERSION } from "./utils.ts";
import { Command } from "commander";
import express from "express";

const program = new Command()
    .option("-p, --port <PORT>")
    .option("--host <HOST>")
    .parse();

const options = program.opts();

const PORT_NUMBER = options.port || 4416;
// This provider is embedded in a desktop application. It must never expose
// its cache or token-minting endpoints to the local network.
const HOST = options.host || "127.0.0.1";

const httpServer = express();
// yt-dlp is a native HTTP client and does not send browser navigation headers.
// Reject them so an unrelated web page cannot use this loopback-only service
// as a cross-site token-minting endpoint.
httpServer.use((request, response, next) => {
    if (request.headers.origin || request.headers.referer) {
        response.status(403).send({ error: "Browser requests are not allowed" });
        return;
    }
    next();
});
httpServer.use(express.json());
httpServer.use(express.urlencoded({ extended: true }));

httpServer
    .listen({ host: HOST, port: PORT_NUMBER }, () => {
        console.log(
            `Started POT server (v${VERSION}) on address ${HOST}:${PORT_NUMBER}`,
        );
    })
    .on("error", (err) => {
        console.error(
            `Could not listen on ${HOST}:${PORT_NUMBER} (Caused by ${strerror(err)})`,
        );
        process.exit(1);
    });

const sessionManager = new SessionManager();
httpServer.post("/get_pot", async (request, response) => {
    const body = request.body || {};
    if (body.data_sync_id)
        return response.status(400).send({
            error: "data_sync_id is deprecated, use content_binding instead",
        });
    if (body.visitor_data)
        return response.status(400).send({
            error: "visitor_data is deprecated, use content_binding instead",
        });
    if (body.disable_innertube)
        return response.status(400).send({
            error: "disable_innertube is deprecated because the /Create endpoint doesn't work anymore",
        });

    const contentBinding: string | undefined = body.content_binding;
    const proxy: string = body.proxy;
    const bypassCache: boolean = body.bypass_cache || false;
    const sourceAddress: string | undefined = body.source_address;
    const disableTlsVerification: boolean =
        body.disable_tls_verification || false;

    try {
        const sessionData = await sessionManager.generatePoToken(
            contentBinding,
            proxy,
            bypassCache,
            sourceAddress,
            disableTlsVerification,
            body.challenge,
            body.innertube_context,
        );

        response.send(sessionData);
    } catch (e) {
        const msg = strerror(e, /*update=*/ true);
        console.error(e.stack);
        response.status(500).send({ error: msg });
    }
});

httpServer.post("/invalidate_caches", async (request, response) => {
    sessionManager.invalidateCaches();
    response.status(204).send();
});

httpServer.post("/invalidate_it", async (request, response) => {
    sessionManager.invalidateIT();
    response.status(204).send();
});

httpServer.get("/ping", async (request, response) => {
    response.send({
        server_uptime: process.uptime(),
        version: VERSION,
    });
});

httpServer.get("/minter_cache", async (request, response) => {
    console.debug(sessionManager.minterCache);
    response.send(Array.from(sessionManager.minterCache.keys()));
});
