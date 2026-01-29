# Parakeet model definitions for declarative downloading
#
# Hashes are SRI format (sha256-...) for use with fetchurl
# URLs point to istupakov HuggingFace repos
#
# Each model requires multiple ONNX files to be downloaded
# and linked together in a directory structure
#
# To update hashes:
#   curl -sL <url> | sha256sum | cut -d' ' -f1 | xxd -r -p | base64 -w0
#   Then prefix with "sha256-"
{
  "parakeet-tdt-0.6b-v2" = {
    repo = "istupakov/parakeet-tdt-0.6b-v2-onnx";
    files = {
      "encoder-model.onnx" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/encoder-model.onnx";
        hash = "sha256-OYe80oF12CnRKIiplqhOj2Kg43TZ/9ZAZiwVFa3GedM=";
      };
      "encoder-model.onnx.data" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/encoder-model.onnx.data";
        hash = "sha256-TatzYtSHTYWWUEWx5BstYd0swPslZxp/az3Ee/EgzEE=";
      };
      "decoder_joint-model.onnx" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/decoder_joint-model.onnx";
        hash = "sha256-y7UqB71wq1tn+EOdSzzYcEsYRntEMLysta2r4VS40ZE=";
      };
      "vocab.txt" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/vocab.txt";
        hash = "sha256-7BgrcN1CETr/bFNyx1ysWMlSRD6yIyL1e71/U5d9SX0=";
      };
      "config.json" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx/resolve/main/config.json";
        hash = "sha256-ZmkDx2uXmMrywhCv1PbNYLCKjb+YAOyNejvA0hSKxGY=";
      };
    };
  };

  "parakeet-tdt-0.6b-v3" = {
    repo = "istupakov/parakeet-tdt-0.6b-v3-onnx";
    files = {
      "encoder-model.onnx" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/encoder-model.onnx";
        hash = "sha256-mKdLIbTMABfB5wMDGaSpb0qVBuUPBwjzpRbQKnfJa7E=";
      };
      "encoder-model.onnx.data" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/encoder-model.onnx.data";
        hash = "sha256-miLTcsUUVcNPE0BdolILrvtxJb0WmBOXVhQj7TLSTzY=";
      };
      "decoder_joint-model.onnx" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/decoder_joint-model.onnx";
        hash = "sha256-6Xjd9miFJxgsEP3i60uDBoQhZImF7yP3qGvnMr6HBsE=";
      };
      "vocab.txt" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/vocab.txt";
        hash = "sha256-1YVEZ56kvGrFY9H1Ret9R0vWz6Rn8KbiwdwcfTfjw10=";
      };
      "config.json" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/config.json";
        hash = "sha256-ZmkDx2uXmMrywhCv1PbNYLCKjb+YAOyNejvA0hSKxGY=";
      };
    };
  };

  "parakeet-tdt-0.6b-v3-int8" = {
    repo = "istupakov/parakeet-tdt-0.6b-v3-onnx";
    files = {
      "encoder-model.int8.onnx" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/encoder-model.int8.onnx";
        hash = "sha256-YTnS+n4bCGCXsnfHFJcl7bq4nMfHrmSyPHQb5AVa/wk=";
      };
      "decoder_joint-model.int8.onnx" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/decoder_joint-model.int8.onnx";
        hash = "sha256-7qdIPuPRowN12u3I7YPjlgyRsJiBISeg2Z0ciXdmenA=";
      };
      "vocab.txt" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/vocab.txt";
        hash = "sha256-1YVEZ56kvGrFY9H1Ret9R0vWz6Rn8KbiwdwcfTfjw10=";
      };
      "config.json" = {
        url = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/config.json";
        hash = "sha256-ZmkDx2uXmMrywhCv1PbNYLCKjb+YAOyNejvA0hSKxGY=";
      };
    };
  };
}
