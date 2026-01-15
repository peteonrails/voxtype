# Whisper model definitions for declarative downloading
#
# Hashes are SRI format (sha256-...) for use with fetchurl
# URLs point to ggerganov/whisper.cpp HuggingFace repo
#
# To update hashes:
#   curl -sL <url> | sha256sum | cut -d' ' -f1 | xxd -r -p | base64 -w0
#   Then prefix with "sha256-"
{
  "tiny" = {
    url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin";
    hash = "sha256-vgfgSOHlma1GNByNKhNWRQl6U4IhZ4t6zdGxkZxuGyE=";
    size = "75 MB";
  };

  "tiny.en" = {
    url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin";
    hash = "sha256-kh5M+Ghv3Zk9zQgaXaW2w2W/3hFi5ysI11rHUomSCx8=";
    size = "75 MB";
  };

  "base" = {
    url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin";
    hash = "sha256-YO1bw90U7qhWST0zQ0m0BXgt3K8AKNS130CINF+6Lv4=";
    size = "142 MB";
  };

  "base.en" = {
    url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin";
    hash = "sha256-oDd5yG3zMjB19eeWyyzlAp8A7Ihp7uP9+4l6/jbG0AI=";
    size = "142 MB";
  };

  "small" = {
    url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin";
    hash = "sha256-G+OpsgY4Z7k35k4ux0gzZKeZF+FX+pjF2UtcH//qmHs=";
    size = "466 MB";
  };

  "small.en" = {
    url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin";
    hash = "sha256-xhONbVjsyDIgl+D5h8MvG+i7ChhTKj+I9zTRu/nEHl0=";
    size = "466 MB";
  };

  "medium" = {
    url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin";
    hash = "sha256-bBTVre5fhjlAN7Tk6LWfFnO2zuEOPPCxG72+55wVYgg=";
    size = "1.5 GB";
  };

  "medium.en" = {
    url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin";
    hash = "sha256-zDfpNHgzjsdwAoGnrDChASiSnrj0J92i6GX6qPbaQ1Y=";
    size = "1.5 GB";
  };

  "large-v3" = {
    url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin";
    hash = "sha256-ZNGCtEC5jVIDxPm9VBVE2ExgUZbE97hF36EfsjWU0eI=";
    size = "3.1 GB";
  };

  "large-v3-turbo" = {
    url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin";
    hash = "sha256-H8cPd0046xaZk6w5Huo1fvR8iHV+9y7llDh5t+jivGk=";
    size = "1.6 GB";
  };
}
