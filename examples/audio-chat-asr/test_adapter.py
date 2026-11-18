import base64
import io
import json
import unittest
from unittest.mock import patch

from adapter import create_app


class AudioChatTest(unittest.TestCase):
    def setUp(self):
        self.client = create_app(8000).test_client()
        connection = patch("adapter.http.client.HTTPConnection")
        self.conn = connection.start().return_value
        self.addCleanup(connection.stop)

    def upload(self):
        return self.client.post("/v1/audio/transcriptions", data={
            "file": (io.BytesIO(b"wav bytes"), "audio.wav", "audio/wav"),
            "model": "example-asr", "language": "en", "prompt": "HTTPS, HTTP/2",
        }, headers={"Authorization": "Bearer example"})

    def reply(self, choice):
        response = io.BytesIO(json.dumps({"choices": [choice]}).encode())
        response.status = 200
        self.conn.getresponse.return_value = response

    def test_audio_url_and_text_response(self):
        self.reply({"finish_reason": "stop", "message": {"content": "hello"}})
        self.assertEqual(self.upload().json, {"text": "hello"})
        method, path, body, headers = self.conn.request.call_args.args
        self.assertEqual((method, path), ("POST", "/v1/chat/completions"))
        self.assertEqual(headers["Authorization"], "Bearer example")
        payload = json.loads(body)
        self.assertEqual(payload["model"], "example-asr")
        prompt, audio = payload["messages"][0]["content"]
        self.assertIn("HTTPS, HTTP/2", prompt["text"])
        self.assertEqual(audio["type"], "audio_url")
        prefix, encoded = audio["audio_url"]["url"].split(",")
        self.assertEqual(prefix, "data:audio/wav;base64")
        self.assertEqual(base64.b64decode(encoded, validate=True), b"wav bytes")

    def test_partial_response_is_not_delivered(self):
        self.reply({"finish_reason": "length", "message": {"content": "partial"}})
        result = self.upload()
        self.assertEqual(result.status_code, 502)
        self.assertNotIn("text", result.json)

    def test_model_error_body_stays_private(self):
        response = io.BytesIO(b"upstream request details")
        response.status = 400
        self.conn.getresponse.return_value = response
        result = self.upload()
        self.assertEqual(result.status_code, 502)
        self.assertNotIn(b"upstream request details", result.data)

    def test_missing_upload_never_reaches_server(self):
        result = self.client.post("/v1/audio/transcriptions", data={"model": "model"})
        self.assertEqual(result.status_code, 400)
        self.conn.request.assert_not_called()


if __name__ == "__main__":
    unittest.main()
