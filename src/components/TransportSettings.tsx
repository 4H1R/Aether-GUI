import { Switch } from "@/components/ui/switch";
import { useConnectionStore } from "@/state/connectionStore";

const INPUT = "h-8 w-full rounded-md bg-black/20 px-2 text-xs text-foreground ring-1 ring-white/10 outline-none focus:ring-primary disabled:opacity-50";

export function TransportSettings() {
  const profile = useConnectionStore((s) => s.profile);
  const status = useConnectionStore((s) => s.status);
  const setTlsFragment = useConnectionStore((s) => s.setTlsFragment);
  const setHttpProxy = useConnectionStore((s) => s.setHttpProxy);
  const setUpstreamProxy = useConnectionStore((s) => s.setUpstreamProxy);
  const locked = status.state !== "Idle" && status.state !== "Error";
  const usesHttp2 = profile.masque_http2 && ["auto", "masque", "mim"].includes(profile.protocol);

  return (
    <div className="flex flex-col gap-3 rounded-md bg-black/10 p-2 ring-1 ring-white/10">
      <label className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
        Fragment HTTP/2 handshake
        <Switch checked={profile.tls_fragment} onCheckedChange={setTlsFragment} disabled={locked || !usesHttp2} aria-label="Fragment HTTP/2 handshake" />
      </label>
      <p className="text-[10px] leading-4 text-muted-foreground">Splits the TLS ClientHello into smaller packets. Available with MASQUE over HTTP/2.</p>
      <label className="flex flex-col gap-1 text-xs text-muted-foreground">
        HTTP CONNECT proxy (optional)
        <input className={INPUT} value={profile.http_proxy} onChange={(e) => setHttpProxy(e.target.value)} disabled={locked} placeholder="127.0.0.1:1820" autoComplete="off" spellCheck={false} />
      </label>
      <p className="text-[10px] leading-4 text-muted-foreground">For apps that support HTTP proxies. Use a different port from SOCKS5. Neither proxy requires authentication; use a loopback address for local access only.</p>
      <label className="flex flex-col gap-1 text-xs text-muted-foreground">
        Upstream proxy (optional, this session only)
        <input className={INPUT} type="password" value={profile.upstream_proxy} onChange={(e) => setUpstreamProxy(e.target.value)} disabled={locked} placeholder="socks5://127.0.0.1:1080" autoComplete="off" spellCheck={false} />
      </label>
      <p className="text-[10px] leading-4 text-muted-foreground">Dial through an existing SOCKS5 or HTTP proxy. Its URL and credentials are never saved.</p>
    </div>
  );
}
