#!/usr/bin/env bash
# Evaluate blueshift's sbpf-linker on the Hopper comparison fixtures.
#
# Runs inside a Linux container (the linker ships no Windows binary), with
# the repository mounted at /w:
#
#   docker run --rm -v "<repo>:/w" -w /w rust:bookworm bash /w/scripts/eval-sbpf-linker.sh
#
# Artifacts land in target/sbpf-linker-eval/{build-sbf,linker-off,linker-fat}.
# They are only sizes until the comparison verifier has run them:
#
#   target/release/framework-verifier --so <artifact> --program-id <id> --case <hello|counter> ...
#
# See audit/research-2026-09-28/SBPF_LINKER_EVALUATION.md for the 2026-09-28
# result (every linker artifact faulted at run time; not adopted).
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null && apt-get install -y -qq curl bzip2 xz-utils ca-certificates >/dev/null
mkdir -p /opt/agave /opt/sbpf-linker /w/target/sbpf-linker-eval
cd /opt/agave
AGAVE_VER="${AGAVE_VER:-v3.1.6}"
echo "== agave $AGAVE_VER"
curl -sSfL "https://github.com/anza-xyz/agave/releases/download/${AGAVE_VER}/solana-release-x86_64-unknown-linux-gnu.tar.bz2" -o agave.tar.bz2
tar xjf agave.tar.bz2
export PATH="/opt/agave/solana-release/bin:$PATH"
cargo-build-sbf --version
echo "== sbpf-linker"
LINKER_VER="${LINKER_VER:-v0.2.2}"
curl -sSfL "https://github.com/blueshift-gg/sbpf-linker/releases/download/${LINKER_VER}/sbpf-linker-x86_64-unknown-linux-musl.tar.gz" -o /opt/sbpf-linker/l.tar.gz
tar xzf /opt/sbpf-linker/l.tar.gz -C /opt/sbpf-linker
find /opt/sbpf-linker -maxdepth 3 -type f -name "sbpf-linker*" | head
LINKER_BIN=$(find /opt/sbpf-linker -type f -name "sbpf-linker" | head -1)
chmod +x "$LINKER_BIN"
# rustc's GNU-ld code path passes lld-only flags the linker's CLI rejects;
# the shim drops them, logs every invocation, and execs the real binary.
mkdir -p /opt/shim
cat > /opt/shim/sbpf-linker <<EOS
#!/usr/bin/env bash
# Whitelist shim: keep inputs, the output, and library paths; turn the
# version script into an explicit export of the entrypoint; drop the rest.
echo "\$@" >> /w/target/sbpf-linker-eval/linker-args.log
if [ "\$1" = "--version" ]; then exec $LINKER_BIN --version; fi
args=(--export entrypoint); next=""
for a in "\$@"; do
  if [ -n "\$next" ]; then args+=("\$next" "\$a"); next=""; continue; fi
  case "\$a" in
    -o|-L) next="\$a";;
    *.o|*.rlib|*.a|*.bc) args+=("\$a");;
    --llvm-args=*) args+=("\$a");;
    *) ;;
  esac
done
echo "EXEC \${args[*]}" >> /w/target/sbpf-linker-eval/linker-args.log
exec $LINKER_BIN "\${args[@]}"
EOS
chmod +x /opt/shim/sbpf-linker
export PATH="/opt/shim:$PATH"
sbpf-linker --version || true
rm -f /w/target/sbpf-linker-eval/linker-args.log
cd /w
export CARGO_HOME=/tmp/cargo-home
mkdir -p "$CARGO_HOME"
cp -r /usr/local/cargo/registry "$CARGO_HOME/" 2>/dev/null || true
OUT=/w/target/sbpf-linker-eval
for fixture in hello counter; do
  crate="${fixture}_hopper"
  manifest="bench/framework-comparison/programs/${fixture}/hopper/Cargo.toml"
  echo "== build-sbf --lto ($fixture)"
  CARGO_TARGET_DIR=/tmp/bsbf CARGO_PROFILE_RELEASE_LTO=fat CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_OPT_LEVEL=3 CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=false \
    cargo-build-sbf --lto --manifest-path "$manifest" --sbf-out-dir "$OUT/build-sbf" 2>&1 | tail -2
  ls -l "$OUT/build-sbf/${crate}.so" | awk '{print "build-sbf --lto:", $5}'
  for lto in off fat; do
    echo "== sbpf-linker lto=$lto ($fixture)"
    RUSTFLAGS="-C linker=sbpf-linker -C linker-flavor=ld -C linker-plugin-lto -C panic=abort -C relocation-model=static -C link-arg=--llvm-args=-bpf-stack-size=4096" \
    CARGO_TARGET_DIR="/tmp/linker-$lto" CARGO_PROFILE_RELEASE_LTO=$lto CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_OPT_LEVEL=3 CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=false \
      cargo +"$(rustup toolchain list | grep -o "[0-9.]*-sbpf-solana-v[0-9.]*" | head -1)" build --release --target sbpf-solana-solana --manifest-path "$manifest" 2>&1 | grep -E "error|note:|Error|Finished" | cut -c1-300 | tail -6
    so=$(find "/tmp/linker-$lto/sbpf-solana-solana/release" -maxdepth 1 -name "*.so" | head -1)
    if [ -n "$so" ]; then mkdir -p "$OUT/linker-$lto"; cp "$so" "$OUT/linker-$lto/${crate}.so"; ls -l "$OUT/linker-$lto/${crate}.so" | awk -v l=$lto '{print "sbpf-linker lto=" l ":", $5}'; else echo "no .so produced"; fi
  done
done
echo SBPF_LINKER_EVAL_DONE
