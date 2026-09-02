// Secrets set via `wrangler secret put` are not visible to `wrangler types`,
// so they are merged into the generated global Env interface here.
interface Env {
	VOXTYPE_API_KEY?: string;
}
