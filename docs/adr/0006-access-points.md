# 0006. Reaching a chamber from outside: access points

Status: accepted as a direction. Not implemented.

## The problem

A chamber runs behind a residential connection, often without a public address and never with a
wish to manage one. Its owner still has a service to expose: a website, an API, a dashboard.

The naive answers are all bad. Port forwarding requires a public address and an inbound rule.
A VPN gives the owner a network, not a URL. Putting the service on a rented box defeats the point
of running it at home in the first place.

## The model

The chamber **dials out** to a Dome access point. The access point holds a public port and forwards
it down that tunnel to a port inside the chamber. Nothing listens inbound on the owner's machine.

```
visitor -> https://shop.example.com   (access point, public)
                 |  SNI or Host: shop.example.com
                 v
           access point port  ->  tunnel  ->  chamber 127.0.0.1:8080
```

Three exposure modes, chosen per published port:

| Mode | Reached at | Requires |
| --- | --- | --- |
| `host` | 127.0.0.1, or the local network with 0.0.0.0 | nothing |
| `access-point` | a domain the access point answers for | an outbound dial |
| `both` | either | both |

## Domains belong to the customer

Two naming modes, and the second is the one that makes this a business rather than a toy:

- **Operator subdomain**: `app.operator.example`. Works immediately, no DNS work.
- **The customer's own domain**: `shop.example.com`, where the customer controls the DNS.

Nothing about how the chamber is built changes. What changes is who is authoritative: for a
custom domain the customer points a record at the access point, and the operator's job becomes
verification rather than permission.

## TLS has two honest answers, and they are not interchangeable

Someone must terminate TLS. Which someone is a per-port decision with real consequences:

| | `passthrough` | `terminate` |
| --- | --- | --- |
| Who holds the key | the chamber | the access point |
| Certificate | the customer's, already in the chamber | issued and renewed by the operator |
| Routing | by SNI, byte-for-byte | by Host header |
| The operator can read traffic | no | yes |
| Works for a custom domain today | yes | requires DNS-01 or ALPN challenge support |

Passthrough is the default for a reason worth stating plainly: it is the only mode where the
answer to \"can the operator read my customers' traffic\" is no. Termination is offered because some
customers cannot manage a certificate at all, and it must then be declared in the blueprint rather
than quietly assumed.

## What the tunnel carries

One connection, many ports. A control stream negotiates and keeps the tunnel alive; each published
port gets a data stream multiplexed over it. The lesson taken from rathole is exactly this shape.

What is deliberately different:

- a published port is **named and scoped** to a chamber, not a raw remote port number, so two
  customers cannot collide and the operator can attach policy to the name
- the tunnel carries **identity**: the chamber proves which release it runs, so an access point
  can refuse a chamber that does not match its seal
- authorization is **per port**: publishing 443 grants nothing for 5432
- the credential is a **key**, treated like a release key, not a shared token in a config file

## What this does not protect

- The access point sees metadata: which chamber connects, when, how much. Any proxy does.
- With `terminate`, the access point sees plaintext. That is the trade, stated up front.
- A compromised chamber still has an outbound tunnel, so it can serve whatever it wants on its
  published ports. Publishing is a capability, not a sandbox boundary.
- The customer's DNS is theirs. Pointing a record elsewhere takes the service away, and no
  mechanism here can prevent that.

## Consequences

- The access point is a public service with credentials, so it needs the same treatment as a
  release: its own threat model, and no path where one tenant's tunnel can address another's.
- @`host`@ remains the default. Exposure is opt-in per port, and a blueprint that says nothing
  publishes nothing.
- Per-port authorization means the blueprint format has to carry ports as structured entries, not
  a list of integers.