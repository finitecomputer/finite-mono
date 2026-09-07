# Official daemon, pinned independently of Hermes. No source or adapter patches.
{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  openssl,
  gmp,
  zlib,
  libffi,
  numactl,
  darwin,
}:
let
  assets = {
    aarch64-darwin = {
      name = "macos-aarch64";
      sha256 = "1ab8d76ad151ffd6166f0c76daa94bb79005b42f8aae22fcdedd0b7b3a034d04";
    };
    x86_64-darwin = {
      name = "macos-x86-64";
      sha256 = "750814cd65d90c8dd2673c7232971e48606bd9833ee610a2554ce36b7ab6be86";
    };
    aarch64-linux = {
      name = "ubuntu-22_04-aarch64";
      sha256 = "91f332edc813cab78a848da7ba75ed91bcb8d09abf1fc92eca74330dc29ec970";
    };
    x86_64-linux = {
      name = "ubuntu-22_04-x86_64";
      sha256 = "5afb1d25efe5ccf564a1ab124bc7f410a7a73171c974a4d8b7f4d8f2e3d62e77";
    };
  };
  asset = assets.${stdenv.hostPlatform.system};
in
stdenv.mkDerivation rec {
  pname = "simplex-chat";
  version = "7.0.2";
  src = fetchurl {
    url = "https://github.com/simplex-chat/simplex-chat/releases/download/v${version}/simplex-chat-${asset.name}";
    inherit (asset) sha256;
  };
  dontUnpack = true;
  dontStrip = true;
  nativeBuildInputs =
    lib.optionals stdenv.isLinux [ autoPatchelfHook ]
    ++ lib.optionals stdenv.isDarwin [
      darwin.cctools
      darwin.autoSignDarwinBinariesHook
    ];
  postFixup = lib.optionalString stdenv.isDarwin ''
    install_name_tool -change /opt/homebrew/opt/openssl@3.0/lib/libcrypto.3.dylib ${lib.getLib openssl}/lib/libcrypto.3.dylib "$out/bin/simplex-chat"
    install_name_tool -change /usr/local/opt/openssl@3.0/lib/libcrypto.3.dylib ${lib.getLib openssl}/lib/libcrypto.3.dylib "$out/bin/simplex-chat"
  '';
  buildInputs = lib.optionals stdenv.isLinux [
    openssl
    gmp
    zlib
    libffi
    numactl
    stdenv.cc.cc.lib
  ];
  installPhase = ''
    install -Dm755 "$src" "$out/bin/simplex-chat"
  '';
  meta = {
    description = "SimpleX Chat daemon for private Agent Runtime connections";
    homepage = "https://simplex.chat";
    license = lib.licenses.agpl3Only;
    platforms = builtins.attrNames assets;
  };
}
