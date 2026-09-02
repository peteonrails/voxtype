/**
 * Everything that knows the shape of Workers AI ASR model responses lives in
 * this module — nothing outside it may touch a Deepgram (or Whisper) response
 * field. The Nova-3 shape is pinned by test/fixtures/nova3-diarized.json,
 * captured from a live call (scripts/capture-fixture.sh).
 */

export const NOVA3_MODEL = "@cf/deepgram/nova-3";
export const WHISPER_MODEL = "@cf/openai/whisper-large-v3-turbo";

/** Languages Nova-3 on Workers AI accepts; anything else is omitted from the call. */
const NOVA3_LANGUAGES = new Set([
	"en", "en-US", "en-AU", "en-GB", "en-IN", "en-NZ",
	"es", "es-419",
	"fr", "fr-CA",
	"de", "de-CH",
	"hi", "ru", "ja", "it", "nl",
	"pt", "pt-BR", "pt-PT",
	"multi",
]);

export function nova3Language(language: string | undefined): string | undefined {
	return language !== undefined && NOVA3_LANGUAGES.has(language) ? language : undefined;
}

/** One word from a diarized Nova-3 response (defensively typed — every field may be absent). */
export interface NovaWord {
	word?: string;
	punctuated_word?: string;
	start?: number;
	end?: number;
	confidence?: number;
	speaker?: number;
}

/** A speaker turn: contiguous words by one speaker without a long gap. */
export interface TurnSegment {
	id: number;
	start: number;
	end: number;
	text: string;
	speaker?: number;
	confidence?: number;
}

export interface TranscriptionResult {
	text: string;
	language?: string;
	duration?: number;
	segments: TurnSegment[];
	words: NovaWord[];
}

export interface TranscribeOptions {
	diarize: boolean;
	language?: string;
	contentType: string;
}

/** Start a new segment when the speaker changes or the inter-word gap exceeds this. */
export const MAX_TURN_GAP_SECS = 3.0;

export function groupWordsIntoTurns(words: NovaWord[], maxGapSecs = MAX_TURN_GAP_SECS): TurnSegment[] {
	const segments: TurnSegment[] = [];
	let current: { words: NovaWord[]; speaker?: number } | null = null;

	const flush = () => {
		if (current === null || current.words.length === 0) return;
		const ws = current.words;
		const confidences = ws.map((w) => w.confidence).filter((c): c is number => typeof c === "number");
		segments.push({
			id: segments.length,
			start: ws[0].start ?? 0,
			end: ws[ws.length - 1].end ?? ws[ws.length - 1].start ?? 0,
			text: ws.map((w) => w.punctuated_word ?? w.word ?? "").join(" ").trim(),
			...(current.speaker !== undefined ? { speaker: current.speaker } : {}),
			...(confidences.length > 0
				? { confidence: confidences.reduce((a, b) => a + b, 0) / confidences.length }
				: {}),
		});
		current = null;
	};

	for (const w of words) {
		if (current !== null) {
			const prev = current.words[current.words.length - 1];
			const gap = (w.start ?? 0) - (prev.end ?? prev.start ?? 0);
			const speakerChanged = w.speaker !== undefined && current.speaker !== undefined && w.speaker !== current.speaker;
			if (speakerChanged || gap > maxGapSecs) flush();
		}
		if (current === null) current = { words: [], speaker: w.speaker };
		current.words.push(w);
	}
	flush();
	return segments;
}

interface Nova3Response {
	results?: {
		channels?: {
			alternatives?: {
				transcript?: string;
				confidence?: number;
				words?: NovaWord[];
			}[];
			detected_language?: string;
		}[];
	};
	metadata?: { duration?: number };
}

/** How to reach Nova-3: the AI binding, plus optional REST-fallback credentials. */
export interface NovaUpstream {
	ai: Ai;
	/** Account id + Workers AI token enable the REST fallback for workerd#5082. */
	accountId?: string;
	apiToken?: string;
}

function novaParams(opts: TranscribeOptions): Record<string, string> {
	const params: Record<string, string> = { punctuate: "true", smart_format: "true" };
	if (opts.diarize) params.diarize = "true";
	const language = nova3Language(opts.language);
	if (language !== undefined) params.language = language;
	return params;
}

/**
 * Run Nova-3 and return the raw, un-normalized model output. Used by the
 * fixture-capture path (`raw=true`); everything else goes through
 * [`transcribeNova3`].
 *
 * Tries the AI binding first, but the binding currently rejects every audio
 * input shape for this model with `5006: required properties at '/audio' are
 * 'body,contentType'` (github.com/cloudflare/workerd#5082 — verified live
 * against base64, number-array, and ReadableStream bodies, with options
 * top-level and nested). On that error it falls back to the documented REST
 * path (binary body + query params), which works, when credentials are
 * configured. When Cloudflare fixes the binding, it takes over automatically.
 */
export async function runNova3Raw(upstream: NovaUpstream, bytes: Uint8Array, opts: TranscribeOptions): Promise<unknown> {
	// Multipart clients often stamp file parts application/octet-stream; the
	// model schema wants a real audio type.
	const contentType = opts.contentType.startsWith("audio/") ? opts.contentType : "audio/wav";
	const params = novaParams(opts);

	try {
		// Same shape Cloudflare's own workers-ai-provider uses for this model.
		// Partner-model schemas aren't in the generated Workers AI types, so the
		// call goes through a narrow cast and responses are parsed defensively
		// against the committed fixture.
		const input: Record<string, unknown> = {
			audio: { body: toBase64(bytes), contentType },
		};
		for (const [k, v] of Object.entries(params)) input[k] = v === "true" ? true : v;
		return await upstream.ai.run(NOVA3_MODEL as Parameters<Ai["run"]>[0], input as never);
	} catch (err) {
		const message = err instanceof Error ? err.message : String(err);
		const isKnownBindingBug = message.includes("5006");
		if (!isKnownBindingBug || upstream.accountId === undefined || upstream.apiToken === undefined) {
			if (isKnownBindingBug) {
				throw new Error(
					`${message} (known Workers AI binding bug for nova-3, workerd#5082; set the CLOUDFLARE_API_TOKEN secret to enable the REST fallback)`,
				);
			}
			throw err;
		}
		console.log(JSON.stringify({ event: "nova3_binding_bug_rest_fallback" }));
		const query = new URLSearchParams(params).toString();
		const url = `https://api.cloudflare.com/client/v4/accounts/${upstream.accountId}/ai/run/${NOVA3_MODEL}?${query}`;
		const response = await fetch(url, {
			method: "POST",
			headers: {
				Authorization: `Bearer ${upstream.apiToken}`,
				"Content-Type": contentType,
			},
			body: bytes,
		});
		const envelope = (await response.json()) as { result?: unknown; success?: boolean; errors?: { message?: string }[] };
		if (!response.ok || envelope.success === false) {
			const detail = envelope.errors?.map((e) => e.message).join("; ") ?? `HTTP ${response.status}`;
			throw new Error(`Workers AI REST call failed: ${detail}`);
		}
		// The REST envelope wraps the model output in `result`; the binding
		// returns it bare. Normalize to bare.
		return envelope.result;
	}
}

/** Run Nova-3 and normalize its response. All Nova-3 shape assumptions live here. */
export async function transcribeNova3(upstream: NovaUpstream, bytes: Uint8Array, opts: TranscribeOptions): Promise<TranscriptionResult> {
	const raw = (await runNova3Raw(upstream, bytes, opts)) as Nova3Response;
	const language = nova3Language(opts.language);

	const channel = raw.results?.channels?.[0];
	const alternative = channel?.alternatives?.[0];
	const text = (alternative?.transcript ?? "").trim();
	const words = alternative?.words ?? [];
	const lastWord = words[words.length - 1];

	return {
		text,
		language: channel?.detected_language ?? language,
		duration: raw.metadata?.duration ?? lastWord?.end,
		segments: words.length > 0
			? groupWordsIntoTurns(words)
			: text.length > 0
				? [{ id: 0, start: 0, end: raw.metadata?.duration ?? 0, text }]
				: [],
		words,
	};
}

interface WhisperResponse {
	text?: string;
}

/** Base64-encode without building one giant intermediate string (stack-safe). */
function toBase64(bytes: Uint8Array): string {
	const chunks: string[] = [];
	const step = 0x8000;
	for (let i = 0; i < bytes.length; i += step) {
		chunks.push(String.fromCharCode(...bytes.subarray(i, i + step)));
	}
	return btoa(chunks.join(""));
}

/** Run whisper-large-v3-turbo (no diarization) and normalize. */
export async function transcribeWhisper(ai: Ai, audio: Uint8Array, language: string | undefined): Promise<TranscriptionResult> {
	// whisper-large-v3-turbo takes base64 audio.
	const input: Record<string, unknown> = {
		audio: toBase64(audio),
		task: "transcribe",
	};
	if (language !== undefined) input.language = language;

	const raw = (await ai.run(WHISPER_MODEL as Parameters<Ai["run"]>[0], input as never)) as WhisperResponse;
	const text = (raw.text ?? "").trim();
	return {
		text,
		language,
		segments: text.length > 0 ? [{ id: 0, start: 0, end: 0, text }] : [],
		words: [],
	};
}
