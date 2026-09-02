import { runNova3Raw, transcribeNova3, transcribeWhisper, type NovaUpstream, type TranscriptionResult } from "./deepgram";
import { errorResponse } from "./errors";

/** Binding plus optional REST-fallback credentials (workerd#5082). */
function novaUpstream(env: Env): NovaUpstream {
	return { ai: env.AI, accountId: env.CLOUDFLARE_ACCOUNT_ID, apiToken: env.CLOUDFLARE_API_TOKEN };
}

/**
 * formData() buffers the body and the audio bytes are buffered again for the
 * AI.run call, so the real ceiling is Worker memory (128 MB), not the 100 MB
 * request-body limit. 50 MB ≈ 26 min of 16 kHz mono s16 WAV — far above any
 * meeting chunk (120 s ≈ 3.8 MB).
 */
const MAX_BODY_BYTES = 50 * 1024 * 1024;

/**
 * POST /v1/audio/transcriptions — OpenAI-compatible, with one voxtype
 * extension: a `diarize=true` form field. Real OpenAI servers ignore unknown
 * form fields, so clients speaking this dialect stay generic OpenAI clients.
 *
 * response_format=json     -> {"text": "..."}            (what voxtype's RemoteTranscriber parses today)
 * response_format=verbose_json -> OpenAI verbose shape + `speaker`/`confidence`
 *                             on segments[] and words[] when diarizing.
 */
export async function handleTranscription(request: Request, env: Env): Promise<Response> {
	const contentLength = Number(request.headers.get("Content-Length") ?? "0");
	if (contentLength > MAX_BODY_BYTES) {
		return errorResponse(413, `Audio too large: request body must be under ${MAX_BODY_BYTES} bytes.`);
	}

	let form: FormData;
	try {
		form = await request.formData();
	} catch {
		return errorResponse(400, "Request body must be multipart/form-data with a 'file' field.");
	}

	const file = form.get("file");
	if (!(file instanceof File)) {
		return errorResponse(400, "Missing required field 'file' (audio to transcribe).");
	}

	const model = str(form.get("model")) ?? "nova-3";
	const language = str(form.get("language"));
	const responseFormat = str(form.get("response_format")) ?? "json";
	const diarize = str(form.get("diarize")) === "true";
	// `prompt` is accepted but ignored in the pilot (Nova-3 keyterm mapping is a follow-up).

	if (responseFormat !== "json" && responseFormat !== "verbose_json") {
		return errorResponse(400, `Unsupported response_format '${responseFormat}': use 'json' or 'verbose_json'.`);
	}

	const useWhisper = model.startsWith("whisper");
	if (useWhisper && diarize) {
		return errorResponse(400, "Diarization requires the nova-3 model; whisper models do not support diarize=true.");
	}

	// Debug extension: raw=true returns the un-normalized Nova-3 model output.
	// Used by scripts/capture-fixture.sh to pin the parser fixture.
	if (str(form.get("raw")) === "true") {
		if (useWhisper) return errorResponse(400, "raw=true is only supported with the nova-3 model.");
		try {
			const raw = await runNova3Raw(novaUpstream(env), new Uint8Array(await file.arrayBuffer()), {
				diarize,
				language,
				contentType: file.type !== "" ? file.type : "audio/wav",
			});
			return Response.json(raw);
		} catch (err) {
			const message = err instanceof Error ? err.message : String(err);
			return errorResponse(502, `Speech-to-text inference failed: ${message}`, "api_error", "inference_failed");
		}
	}

	const started = Date.now();
	let result: TranscriptionResult;
	try {
		if (useWhisper) {
			result = await transcribeWhisper(env.AI, new Uint8Array(await file.arrayBuffer()), language);
		} else {
			const contentType = file.type !== "" ? file.type : "audio/wav";
			result = await transcribeNova3(novaUpstream(env), new Uint8Array(await file.arrayBuffer()), { diarize, language, contentType });
		}
	} catch (err) {
		const message = err instanceof Error ? err.message : String(err);
		console.log(JSON.stringify({ event: "inference_error", model, message }));
		return errorResponse(502, `Speech-to-text inference failed: ${message}`, "api_error", "inference_failed");
	}

	// Never log transcript text or audio — durations and sizes only.
	console.log(
		JSON.stringify({
			event: "transcription",
			model: useWhisper ? "whisper-large-v3-turbo" : "nova-3",
			diarize,
			response_format: responseFormat,
			audio_bytes: file.size,
			audio_secs: result.duration ?? null,
			elapsed_ms: Date.now() - started,
		}),
	);

	if (responseFormat === "json") {
		return Response.json({ text: result.text });
	}
	return Response.json({
		task: "transcribe",
		language: result.language ?? "en",
		duration: result.duration ?? 0,
		text: result.text,
		segments: result.segments,
		words: result.words,
	});
}

function str(value: unknown): string | undefined {
	return typeof value === "string" && value.length > 0 ? value : undefined;
}
