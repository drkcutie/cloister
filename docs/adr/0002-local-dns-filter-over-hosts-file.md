# Local DNS filter instead of only the hosts file

On every desktop, the root enforcement service runs a local DNS filter that is the system's only resolver. It blocks whole domains together with their subdomains and known related domains (e.g. `youtube.com` → `*.youtube.com`, `youtu.be`, `*.googlevideo.com`). A firewall (`pf`, Windows Firewall or `nftables`) blocks the resolved IPs and stops DNS from going anywhere else. We chose this over SelfControl's approach of hosts file plus firewall because the hosts file can only list exact names, so every subdomain or mirror slips through. The hosts file stays as a fallback in case the filter fails. Android's VPN filter works the same way, so both platforms share one model of what "blocked" means.

## Considered options

- **Hosts file + firewall only (SelfControl).** Simpler, but subdomains get through.
- **Managed browser policies.** Rejected: SelfControl-level strictness is the goal, and policies only cover supported browsers.
- **Blocking VPN, Tor and proxy tools.** Rejected: we deliberately leave out anyone who works hard at bypassing it. The target is friction against impulses, not resistance to determined circumvention.
