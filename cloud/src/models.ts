/** GET /v1/models — static OpenAI-style model list. */
export function handleModels(): Response {
	return Response.json({
		object: "list",
		data: [
			{ id: "nova-3", object: "model", created: 1756684800, owned_by: "deepgram" },
			{ id: "whisper-large-v3-turbo", object: "model", created: 1756684800, owned_by: "openai" },
		],
	});
}
