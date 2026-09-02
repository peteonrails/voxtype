// Secrets set via `wrangler secret put` are not visible to `wrangler types`,
// so they are merged into the generated global Env interface here.
interface Env {
	VOXTYPE_API_KEY?: string;
	/** Workers AI-scoped token enabling the nova-3 REST fallback (workerd#5082). */
	CLOUDFLARE_API_TOKEN?: string;
}
