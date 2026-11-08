{ pkgs }:

pkgs.stdenv.mkDerivation {
  pname = "onnxruntime-webgpu";
  version = "1.24.2";

  # Keep this distribution in sync with ort-sys's WebGPU download table.
  src = pkgs.fetchurl {
    url = "https://cdn.pyke.io/0/pyke:ort-rs/ms@1.24.2/x86_64-unknown-linux-gnu+wgpu.tar.lzma2";
    sha256 = "e9aa41101eacde0bf8f832f28c06a8bf3d0f7896a463e0b2d3550563583262b9";
  };

  nativeBuildInputs = [ pkgs.autoPatchelfHook pkgs.xz ];
  buildInputs = [ pkgs.stdenv.cc.cc.lib ];
  appendRunpaths = [ "${pkgs.vulkan-loader}/lib" ];

  unpackPhase = ''
    runHook preUnpack
    xz --decompress --format=raw --lzma2=dict=64MiB --stdout "$src" | tar -x
    runHook postUnpack
  '';

  dontBuild = true;

  installPhase = ''
    runHook preInstall
    install -Dm644 libonnxruntime.a "$out/lib/libonnxruntime.a"
    install -Dm755 libwebgpu_dawn.so "$out/lib/libwebgpu_dawn.so"
    mkdir -p "$out/share/licenses/onnxruntime-webgpu"
    cp ${../packaging/licenses/webgpu}/* "$out/share/licenses/onnxruntime-webgpu/"
    runHook postInstall
  '';

  meta = with pkgs.lib; {
    description = "ONNX Runtime with the native WebGPU provider and matching Dawn library";
    homepage = "https://onnxruntime.ai";
    license = [ licenses.mit licenses.bsd3 ];
    sourceProvenance = [ sourceTypes.binaryNativeCode ];
    platforms = [ "x86_64-linux" ];
  };
}
