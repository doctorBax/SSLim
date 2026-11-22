#!/usr/bin/env bash
set -euo pipefail
umask 077
export LC_ALL=C
dir=/etc/sslim
exec 9>"$dir/renew.lock"
flock -n 9 || { echo 'Certificate renewal is already running.' >&2; exit 1; }
for path in hostname ca.pem ca.key server.pem server.key; do
  [[ -f $dir/$path && ! -L $dir/$path ]] || { echo "Missing or unsafe $dir/$path; CA migration may be required." >&2; exit 1; }
done
host=$(cat "$dir/hostname")
[[ $host =~ ^[A-Za-z0-9][A-Za-z0-9.-]*$ ]] || { echo 'Invalid configured hostname.' >&2; exit 1; }
verify_name=(-verify_hostname "$host")
san=DNS:$host
if [[ $host =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then verify_name=(-verify_ip "$host"); san=IP:$host; fi
pubkey_cert() { openssl x509 -in "$1" -pubkey -noout | openssl pkey -pubin -outform DER | sha256sum; }
pubkey_key() { openssl pkey -in "$1" -pubout -outform DER | sha256sum; }
[[ $(pubkey_cert "$dir/ca.pem") == "$(pubkey_key "$dir/ca.key")" ]] || { echo 'CA certificate and key do not match.' >&2; exit 1; }
[[ $(pubkey_cert "$dir/server.pem") == "$(pubkey_key "$dir/server.key")" ]] || { echo 'Server certificate and key do not match.' >&2; exit 1; }
openssl verify -no_check_time -CAfile "$dir/ca.pem" "${verify_name[@]}" "$dir/server.pem" >/dev/null || { echo 'Current server certificate, CA or SAN is invalid.' >&2; exit 1; }
if openssl x509 -in "$dir/server.pem" -checkend 2592000 -noout >/dev/null; then exit 0; fi
# A new leaf must never outlive its trust anchor.
openssl x509 -in "$dir/ca.pem" -checkend 34560000 -noout >/dev/null || { echo 'CA expires too soon; migrate trust before renewal.' >&2; exit 1; }
tmp=$(mktemp -d "$dir/.renew.XXXXXXXX")
replaced=false
cleanup() {
  code=$?
  trap - EXIT
  if $replaced; then
    echo 'Renewal interrupted or failed; restoring previous certificate.' >&2
    if cp -p "$dir/server.pem.previous" "$tmp/restore.pem" && mv -f "$tmp/restore.pem" "$dir/server.pem"; then
      systemctl restart sslim.service || echo 'Rollback restart failed; inspect sslim.service immediately.' >&2
    else
      echo 'Certificate rollback failed; previous certificate is in server.pem.previous.' >&2
    fi
  fi
  rm -rf -- "$tmp"
  exit "$code"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
openssl req -new -key "$dir/server.key" -subj "/CN=$host" -out "$tmp/server.csr"
printf 'subjectAltName=%s\nextendedKeyUsage=serverAuth\n' "$san" > "$tmp/extensions"
openssl x509 -req -in "$tmp/server.csr" -CA "$dir/ca.pem" -CAkey "$dir/ca.key" -set_serial "0x$(openssl rand -hex 16)" -out "$tmp/server.pem" -days 365 -extfile "$tmp/extensions"
openssl verify -CAfile "$dir/ca.pem" "${verify_name[@]}" "$tmp/server.pem" >/dev/null
[[ $(pubkey_cert "$tmp/server.pem") == "$(pubkey_key "$dir/server.key")" ]]
cp -p "$dir/server.pem" "$tmp/previous.pem"
mv -f "$tmp/previous.pem" "$dir/server.pem.previous"
chmod 0600 "$tmp/server.pem"
replaced=true
mv -f "$tmp/server.pem" "$dir/server.pem"
if systemctl restart sslim.service && systemctl is-active --quiet sslim.service && \
   timeout 15 bash -c 'until (echo > /dev/tcp/127.0.0.1/9443) 2>/dev/null; do sleep 0.25; done' && \
   timeout 10 openssl s_client -connect 127.0.0.1:9443 -CAfile "$dir/ca.pem" -verify_return_error "${verify_name[@]}" </dev/null > "$tmp/served" 2>/dev/null && \
   [[ $(pubkey_cert "$tmp/served") == "$(pubkey_cert "$dir/server.pem")" ]] && \
   [[ $(openssl x509 -in "$tmp/served" -noout -serial) == "$(openssl x509 -in "$dir/server.pem" -noout -serial)" ]]; then
  replaced=false
  echo 'Server certificate renewed and verified.'
else
  exit 1
fi
