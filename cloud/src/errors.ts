/**
 * OpenAI-style error envelope. voxtype's RemoteTranscriber surfaces the raw
 * status + body verbatim in its error message, so the message must stand on
 * its own for a human reading daemon logs.
 */
export interface OpenAiError {
	error: {
		message: string;
		type: string;
		code: string | null;
	};
}

export function errorResponse(status: number, message: string, type = "invalid_request_error", code: string | null = null): Response {
	const body: OpenAiError = { error: { message, type, code } };
	return Response.json(body, { status });
}

export function unauthorized(): Response {
	return errorResponse(401, "Invalid or missing API key. Pass it as 'Authorization: Bearer <key>'.", "authentication_error", "invalid_api_key");
}

export function notFound(message = "Not found."): Response {
	return errorResponse(404, message, "invalid_request_error", "not_found");
}

export function methodNotAllowed(): Response {
	return errorResponse(405, "Method not allowed.", "invalid_request_error", "method_not_allowed");
}
