# default.nix

{ stdenv, fetchurl, lib, ... }:

let
  version = "0.3.15";
  owner = "scottidler";
  repo = "aka";
  suffix = "linux"; # Adjust based on the target OS, e.g., "macos" for macOS builds

  # URL and sha256 for the release tarball
  tarball = fetchurl {
    url = "https://github.com/${owner}/${repo}/releases/download/v${version}/aka-v${version}-${suffix}.tar.gz";
    sha256 = "082wk31b2ybs63rxib7ym54jly4ywwiyiz7shnxda18hl0ijsrxd"; # Use `nix-prefetch-url` to obtain this
  };

in stdenv.mkDerivation rec {
  pname = "aka";
  inherit version;

  src = tarball;

  dontBuild = true;

  unpackPhase = ''
    mkdir -p $out/bin
    mkdir -p $out/share/zsh/site-functions
    tar -xzf $src -C $out/bin --strip-components=0
  '';

  installPhase = ''
    mv $out/bin/_aka $out/share/zsh/site-functions/
  '';

  meta = with lib; {
    description = "Aka - a friendly command aliasing program with Zsh integration";
    homepage = "https://github.com/${owner}/${repo}";
    license = licenses.mit;
    platforms = platforms.linux ++ platforms.darwin;
    maintainers = with maintainers; [ maintainers.saidler ];
  };
}

