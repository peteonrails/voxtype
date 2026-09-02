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

describe("transcribeNova3 against the pinned fixture", () => {
	const stubAi = { run: async () => fixture } as unknown as Ai;

	it("extracts transcript, duration, and speaker turns", async () => {
		const result = await transcribeNova3(stubAi, new Uint8Array(0), {
			diarize: true,
			contentType: "audio/wav",
		});
		expect(result.text).toBe("So the deploy failed twice. Yeah, I saw that in the logs.");
		expect(result.duration).toBeCloseTo(8.42);
		expect(result.language).toBe("en");
		expect(result.segments).toHaveLength(2);
		expect(result.segments[0].speaker).toBe(0);
		expect(result.segments[0].text).toBe("So the deploy failed twice.");
		expect(result.segments[1].speaker).toBe(1);
		expect(result.segments[1].text).toBe("Yeah, I saw that in the logs.");
	});

	it("falls back to a single segment when words are missing", async () => {
		const bare = { results: { channels: [{ alternatives: [{ transcript: "hello there" }] }] } };
		const ai = { run: async () => bare } as unknown as Ai;
		const result = await transcribeNova3(ai, new Uint8Array(0), { diarize: false, contentType: "audio/wav" });
		expect(result.text).toBe("hello there");
		expect(result.segments).toHaveLength(1);
		expect(result.segments[0].speaker).toBeUndefined();
	});

	it("returns empty segments for an empty response", async () => {
		const ai = { run: async () => ({}) } as unknown as Ai;
		const result = await transcribeNova3(ai, new Uint8Array(0), { diarize: false, contentType: "audio/wav" });
		expect(result.text).toBe("");
		expect(result.segments).toHaveLength(0);
	});
});
