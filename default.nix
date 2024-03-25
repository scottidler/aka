# default.nix
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
    mkdir -p $out/share/aka
    cp $src/HOME/.expand-aka $out/share/aka/.expand-aka
  '';

  meta = with lib; {
    description = "A description of the aka program";
    homepage = "https://github.com/your-github-username/aka";
    license = licenses.mit;
    maintainers = with maintainers; [ your-maintainer-name ];
  };
}
