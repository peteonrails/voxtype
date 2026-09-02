import { describe, expect, it } from "vitest";
import { groupWordsIntoTurns, nova3Language, transcribeNova3, type NovaWord } from "../src/deepgram";
import fixture from "./fixtures/nova3-diarized.json";

function word(w: Partial<NovaWord>): NovaWord {
	return { word: "x", start: 0, end: 0.1, confidence: 0.9, ...w };
}

describe("groupWordsIntoTurns", () => {
	it("splits on speaker change", () => {
		const turns = groupWordsIntoTurns([
			word({ word: "hello", start: 0, end: 0.5, speaker: 0 }),
			word({ word: "there", start: 0.5, end: 1.0, speaker: 0 }),
			word({ word: "hi", start: 1.2, end: 1.5, speaker: 1 }),
		]);
		expect(turns).toHaveLength(2);
		expect(turns[0].speaker).toBe(0);
		expect(turns[0].text).toBe("hello there");
		expect(turns[1].speaker).toBe(1);
		expect(turns[1].text).toBe("hi");
	});

	it("splits on a gap longer than the max, same speaker", () => {
		const turns = groupWordsIntoTurns([
			word({ word: "one", start: 0, end: 0.5, speaker: 0 }),
			word({ word: "two", start: 4.0, end: 4.5, speaker: 0 }),
		]);
		expect(turns).toHaveLength(2);
		expect(turns.every((t) => t.speaker === 0)).toBe(true);
	});

	it("does not split on a short gap", () => {
		const turns = groupWordsIntoTurns([
			word({ word: "one", start: 0, end: 0.5, speaker: 0 }),
			word({ word: "two", start: 2.9, end: 3.2, speaker: 0 }),
		]);
		expect(turns).toHaveLength(1);
	});

	it("prefers punctuated_word and averages confidence", () => {
		const turns = groupWordsIntoTurns([
			word({ word: "hello", punctuated_word: "Hello,", start: 0, end: 0.4, confidence: 0.8, speaker: 0 }),
			word({ word: "world", punctuated_word: "world.", start: 0.4, end: 0.9, confidence: 1.0, speaker: 0 }),
		]);
		expect(turns[0].text).toBe("Hello, world.");
		expect(turns[0].confidence).toBeCloseTo(0.9);
		expect(turns[0].start).toBe(0);
		expect(turns[0].end).toBe(0.9);
	});

	it("handles undiarized words (no speaker field) as one turn", () => {
		const turns = groupWordsIntoTurns([
			word({ word: "just", start: 0, end: 0.3 }),
			word({ word: "text", start: 0.3, end: 0.6 }),
		]);
		expect(turns).toHaveLength(1);
		expect(turns[0].speaker).toBeUndefined();
	});

	it("returns no turns for no words", () => {
		expect(groupWordsIntoTurns([])).toHaveLength(0);
	});

	it("ids are sequential", () => {
		const turns = groupWordsIntoTurns([
			word({ word: "a", start: 0, end: 0.1, speaker: 0 }),
			word({ word: "b", start: 0.2, end: 0.3, speaker: 1 }),
			word({ word: "c", start: 0.4, end: 0.5, speaker: 0 }),
		]);
		expect(turns.map((t) => t.id)).toEqual([0, 1, 2]);
	});
});

describe("nova3Language", () => {
	it("passes through supported languages", () => {
		expect(nova3Language("en")).toBe("en");
		expect(nova3Language("pt-BR")).toBe("pt-BR");
		expect(nova3Language("multi")).toBe("multi");
	});
	it("omits unsupported languages", () => {
		expect(nova3Language("zh")).toBeUndefined();
		expect(nova3Language(undefined)).toBeUndefined();
	});
});

describe("transcribeNova3 against the pinned fixture (live-captured 2026-09-02)", () => {
	const stubAi = { run: async () => fixture } as unknown as Ai;

	it("extracts transcript, duration, and speaker-tagged turns", async () => {
		const result = await transcribeNova3({ ai: stubAi }, new Uint8Array(0), {
			diarize: true,
			contentType: "audio/wav",
		});
		expect(result.text).toBe(
			"So the deploy failed twice last night. I think the migration step is timing out before the database comes up. Yeah. I saw that in the logs this morning. Let's add a health check before the migration runs and try again.",
		);
		// The live response carries no metadata.duration; it falls back to the
		// last word's end time.
		expect(result.duration).toBeCloseTo(14.96);
		expect(result.words).toHaveLength(41);
		expect(result.segments.length).toBeGreaterThanOrEqual(1);
		// Workers AI's nova-3 currently tags every word speaker 0 even on
		// multi-speaker audio (upstream diarization defect, observed live on
		// Deepgram's own two-speaker demo files); the fixture reflects that.
		// The turn-grouping tests above cover real multi-speaker word arrays.
		expect(result.segments.every((s) => s.speaker === 0)).toBe(true);
		expect(result.segments.map((s) => s.text).join(" ")).toBe(result.text);
	});

	it("falls back to a single segment when words are missing", async () => {
		const bare = { results: { channels: [{ alternatives: [{ transcript: "hello there" }] }] } };
		const ai = { run: async () => bare } as unknown as Ai;
		const result = await transcribeNova3({ ai }, new Uint8Array(0), { diarize: false, contentType: "audio/wav" });
		expect(result.text).toBe("hello there");
		expect(result.segments).toHaveLength(1);
		expect(result.segments[0].speaker).toBeUndefined();
	});

	it("returns empty segments for an empty response", async () => {
		const ai = { run: async () => ({}) } as unknown as Ai;
		const result = await transcribeNova3({ ai }, new Uint8Array(0), { diarize: false, contentType: "audio/wav" });
		expect(result.text).toBe("");
		expect(result.segments).toHaveLength(0);
	});

	it("falls back to REST on the 5006 binding bug and unwraps the envelope", async () => {
		const ai = {
			run: async () => {
				throw new Error("5006: Error: required properties at '/audio' are 'body,contentType'");
			},
		} as unknown as Ai;
		const calls: { url: string; init: RequestInit }[] = [];
		const originalFetch = globalThis.fetch;
		globalThis.fetch = (async (url: string | URL | Request, init?: RequestInit) => {
			calls.push({ url: String(url), init: init ?? {} });
			return Response.json({ result: fixture, success: true });
		}) as typeof fetch;
		try {
			const result = await transcribeNova3({ ai, accountId: "acct123", apiToken: "tok" }, new Uint8Array([1, 2, 3]), {
				diarize: true,
				language: "en",
				contentType: "audio/wav",
			});
			expect(result.words).toHaveLength(41);
			expect(calls).toHaveLength(1);
			const { url, init } = calls[0];
			// Mirrors the verified-working curl: binary body + query params.
			expect(url).toBe(
				"https://api.cloudflare.com/client/v4/accounts/acct123/ai/run/@cf/deepgram/nova-3?punctuate=true&smart_format=true&diarize=true&language=en",
			);
			expect((init.headers as Record<string, string>).Authorization).toBe("Bearer tok");
			expect((init.headers as Record<string, string>)["Content-Type"]).toBe("audio/wav");
			expect(init.body).toBeInstanceOf(Uint8Array);
		} finally {
			globalThis.fetch = originalFetch;
		}
	});

	it("surfaces the workerd#5082 hint when the binding rejects and no fallback is configured", async () => {
		const ai = {
			run: async () => {
				throw new Error("5006: Error: required properties at '/audio' are 'body,contentType'");
			},
		} as unknown as Ai;
		await expect(transcribeNova3({ ai }, new Uint8Array(0), { diarize: false, contentType: "audio/wav" })).rejects.toThrow(
			/workerd#5082/,
		);
	});
});
