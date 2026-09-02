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

/** Run Nova-3 and normalize its response. All Nova-3 shape assumptions live here. */
export async function transcribeNova3(ai: Ai, audio: ReadableStream | Uint8Array, opts: TranscribeOptions): Promise<TranscriptionResult> {
	const input: Record<string, unknown> = {
		audio: { body: audio, contentType: opts.contentType },
		punctuate: true,
		smart_format: true,
	};
	if (opts.diarize) input.diarize = true;
	const language = nova3Language(opts.language);
	if (language !== undefined) input.language = language;

	// Partner-model input/output schemas aren't covered by the generated Workers
	// AI types, so the call goes through a narrow local cast and the response is
	// parsed defensively against the committed fixture.
	const raw = (await ai.run(NOVA3_MODEL as Parameters<Ai["run"]>[0], input as never)) as Nova3Response;

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
