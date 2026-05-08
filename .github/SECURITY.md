# Security

If you believe you have found a **security vulnerability** in this repository, please report it **privately** so it can be investigated before wider disclosure.

**Do not** open a public GitHub Issue for an undisclosed vulnerability.

## How to report

1. **Preferred (if enabled):** Use GitHub **Security** → **Report a vulnerability** for this repository ([private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)).

2. **Otherwise:** Contact **Figure Markets, Inc.** through the same channels you would use for matters under the [LICENSE](../LICENSE) (e.g. commercial or licensing inquiries), with a subject line that includes **Security** and the name of this repository.

## What to include

- A short description of the issue and its impact  
- Affected contract(s), paths, or release / commit if known  
- Steps to reproduce, or a proof-of-concept where safe to share  
- Whether you believe any **mainnet** deployment is affected (if known)  

Encrypted contact (e.g. PGP) may be published by Figure Markets separately; use that if offered.

## Scope (in scope)

- CosmWasm contracts and on-chain logic in this repository (pool, repo token, oracle, shared library)  
- Issues that could lead to loss of funds, unauthorized mint/burn/transfer, incorrect accounting, or privilege escalation **in contract code**  

## Out of scope (examples)

- Theoretical issues with no practical path to exploit on a realistic deployment  
- Third-party dependencies, validators, wallets, or RPC infrastructure — report those to the respective maintainers unless the issue is in **this repository’s** integration code  
- Social engineering, phishing, or physical attacks  
- Spam, denial-of-service, or automated scanner output without a clear, reproducible exploit  

## What to expect

- We aim to **acknowledge** good-faith reports in a reasonable time; timing depends on severity and internal process.  
- This policy does **not** promise a public bug bounty or payment unless Figure Markets states otherwise in writing.  
- Public discussion of a confirmed issue may be coordinated after a fix or mitigation is available, at the Licensor’s discretion.  

## License reminder

Use and redistribution of this code are governed by the [LICENSE](../LICENSE) (Business Source License 1.1). Reporting a vulnerability does not change those terms.
