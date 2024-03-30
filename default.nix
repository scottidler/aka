{ lib, stdenv, fetchFromGitHub, rustPlatform, installShellFiles }:

rustPlatform.buildRustPackage rec {
  pname = "aka";
  version = "0.3.6";

  src = fetchFromGitHub {
    owner = "scottidler";
    repo = pname;
    rev = "v${version}";
    sha256 = "sha256-5m+boXqZZ6iYtvz9icsq0hMmTMXrvR2ThgqKckarRL4=";
  };

  cargoSha256 = "sha256-mc5FeU+OIqsOIyid2Y8QBpHVDvwXMU62j4Tj+QPLwiM=";

  nativeBuildInputs = [ installShellFiles ];

  postInstall = ''
    install -D ./target/x86_64-unknown-linux-gnu/release/aka $out/bin/aka
    install -D ${src}/HOME/.expand-aka $out/share/aka/.expand-aka
  '';

  meta = with lib; {
    description = "[a]lso [k]nown [a]s: an aliasing program";
    homepage = "https://github.com/scottidler/aka";
    license = licenses.mit;
    maintainers = with maintainers; [ lib.maintainers.scottidler ];
  };
}
