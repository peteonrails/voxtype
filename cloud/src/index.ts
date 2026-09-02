import { transcribeNova3 } from "./deepgram";
import { errorResponse, methodNotAllowed, notFound, unauthorized } from "./errors";
import { handleModels } from "./models";
import { handleTranscription } from "./transcriptions";

/**
 * Look up the set of valid API keys. The pilot has exactly one, from a Worker
 * secret; this seam is where per-user keys in KV/D1 drop in later without
 * touching the auth check itself.
 */
async function validKeys(env: Env): Promise<string[]> {
	return env.VOXTYPE_API_KEY !== undefined && env.VOXTYPE_API_KEY !== "" ? [env.VOXTYPE_API_KEY] : [];
}

/** Constant-time comparison via SHA-256 digests (no length leak). */
export async function keyMatches(presented: string, valid: string): Promise<boolean> {
	const encoder = new TextEncoder();
	const [a, b] = await Promise.all([
		crypto.subtle.digest("SHA-256", encoder.encode(presented)),
		crypto.subtle.digest("SHA-256", encoder.encode(valid)),
	]);
	return crypto.subtle.timingSafeEqual(a, b);
}

async function authenticate(request: Request, env: Env): Promise<boolean> {
	const header = request.headers.get("Authorization") ?? "";
	if (!header.startsWith("Bearer ")) return false;
	const presented = header.slice("Bearer ".length).trim();
	if (presented === "") return false;
	for (const valid of await validKeys(env)) {
		if (await keyMatches(presented, valid)) return true;
	}
	return false;
}

export default {
	async fetch(request, env, _ctx): Promise<Response> {
		const url = new URL(request.url);
		try {
			if (!(await authenticate(request, env))) {
				return unauthorized();
			}

			switch (url.pathname) {
				case "/v1/audio/transcriptions":
					return request.method === "POST" ? await handleTranscription(request, env) : methodNotAllowed();
				case "/v1/models":
					return request.method === "GET" ? handleModels() : methodNotAllowed();
				// Reserved routes: claimed in the URL space now, built post-pilot.
				case "/v1/chat/completions":
					return notFound("Not yet available: summarization is planned for a future Voxtype Cloud release.");
				case "/v1/realtime":
					return notFound("Not yet available: realtime transcription is planned for a future Voxtype Cloud release.");
				default:
					return notFound(`Unknown path '${url.pathname}'.`);
			}
		} catch (err) {
			const message = err instanceof Error ? err.message : String(err);
			console.log(JSON.stringify({ event: "unhandled_error", path: url.pathname, message }));
			return errorResponse(500, "Internal error.", "api_error", "internal_error");
		}
	},
} satisfies ExportedHandler<Env>;

// Re-exported so the fixture-capture script's sibling test can exercise the
// same code path the Worker uses.
export { transcribeNova3 };
