# /// script
# requires-python = ">=3.10"
# dependencies = ["Flask>=3.1,<4"]
# ///
"""Let Voxtype remote mode use a loopback audio chat-completions server."""

import argparse
import base64
import http.client
import json

from flask import Flask, request


def create_app(upstream_port):
    app = Flask(__name__)
    app.config["MAX_CONTENT_LENGTH"] = 32 * 1024 * 1024

    @app.post("/v1/audio/transcriptions")
    def transcribe():
        audio = request.files.get("file")
        model = request.form.get("model")
        if audio is None or not model or audio.mimetype != "audio/wav":
            return {"error": "Expected a model and a WAV upload"}, 400

        prompt = "Transcribe the following audio. Return only the transcript."
        language = request.form.get("language")
        if language and language != "auto":
            prompt += f" The spoken language is {language}."
        context = request.form.get("prompt")
        if context:
            prompt += f"\nTranscription context: {context}"

        wav = base64.b64encode(audio.read()).decode("ascii")
        payload = {
            "model": model,
            "temperature": 0,
            "chat_template_kwargs": {"enable_thinking": False},
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": prompt},
                {"type": "audio_url", "audio_url": {"url": f"data:audio/wav;base64,{wav}"}},
            ]}],
        }
        headers = {"Content-Type": "application/json"}
        if "Authorization" in request.headers:
            headers["Authorization"] = request.headers["Authorization"]
        conn = http.client.HTTPConnection("127.0.0.1", upstream_port, timeout=120)
        try:
            conn.request("POST", "/v1/chat/completions", json.dumps(payload), headers)
            response = conn.getresponse()
            if response.status != 200:
                return {"error": f"Audio model server returned HTTP {response.status}"}, 502
            choice = json.load(response)["choices"][0]
            text = choice["message"]["content"]
            if choice.get("finish_reason") != "stop" or not isinstance(text, str):
                return {"error": "Audio model returned an incomplete transcript"}, 502
            return {"text": text}
        except (OSError, http.client.HTTPException, ValueError, KeyError, IndexError, TypeError):
            return {"error": "Could not read a completion from the audio model server"}, 502
        finally:
            conn.close()

    return app


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8011)
    parser.add_argument("--upstream-port", type=int, default=8000)
    args = parser.parse_args()
    create_app(args.upstream_port).run(host="127.0.0.1", port=args.port)
