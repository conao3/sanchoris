import { Form } from 'react-aria-components';
import { Button } from '../components/Button';
import { SSOButton } from '../components/SSOButton';
import { TextField } from '../components/TextField';
import { Wordmark } from '../components/Wordmark';

export function LoginPage() {
  return (
    <main className="flex min-h-screen flex-col lg:flex-row">
      <BrandPanel />
      <AuthPanel />
    </main>
  );
}

function BrandPanel() {
  return (
    <aside className="flex flex-1 flex-col justify-between bg-surface-soft px-16 py-14">
      <header className="flex items-center justify-between">
        <Wordmark size="md" />
        <StatusPill />
      </header>

      <div className="flex flex-col gap-7">
        <p className="font-mono text-sm font-medium uppercase tracking-wider text-muted-soft">
          The gateway for AI software delivery
        </p>
        <h1 className="font-serif text-hero font-medium leading-heading text-ink">
          Coordinate the work.
          <br />
          Prove the work.
          <br />
          <span className="text-primary">Ship the work.</span>
        </h1>
        <p className="max-w-[480px] text-lg leading-body text-body">
          One front door for every agent and channel,
          <br />
          tied to one verifiable ship log.
        </p>
      </div>
    </aside>
  );
}

function StatusPill() {
  return (
    <span className="inline-flex items-center gap-2 rounded-pill bg-cream-strong px-3 py-1 font-mono text-sm text-muted">
      <span className="size-1.5 rounded-full bg-success" aria-hidden="true" />
      all gateways operational
    </span>
  );
}

function AuthPanel() {
  return (
    <section className="flex w-full flex-col justify-between bg-canvas px-16 py-[60px] lg:w-[560px] lg:flex-none">
      <div className="flex flex-col gap-7">
        <div className="flex items-center justify-between">
          <a href="/" className="text-base text-muted">
            ← Back to home
          </a>
          <div className="flex items-center gap-3 text-base">
            <span className="text-muted">Already have an account?</span>
            <a href="/signin" className="font-medium text-primary">
              Sign in →
            </a>
          </div>
        </div>

        <div className="flex flex-col gap-4">
          <p className="font-mono text-sm font-medium uppercase tracking-wider text-muted-soft">
            Get started
          </p>
          <h1 className="font-serif text-h1 font-medium leading-heading text-ink">
            Create your
            <br />
            gateway.
          </h1>
          <div className="flex flex-col gap-1 text-md leading-body text-body">
            <p>Start coordinating your AI delivery in under two minutes.</p>
            <p>Email + password or continue with Google — no credit card.</p>
          </div>
        </div>

        <SSOButton iconLabel="G" iconTone="dark" label="Continue with Google" />

        <div className="flex items-center gap-3.5">
          <span className="h-px flex-1 bg-hairline" aria-hidden="true" />
          <span className="font-mono text-sm font-medium uppercase tracking-wider text-muted-soft">
            or use email
          </span>
          <span className="h-px flex-1 bg-hairline" aria-hidden="true" />
        </div>

        <Form
          className="flex flex-col gap-3.5"
          onSubmit={(e) => {
            e.preventDefault();
          }}
        >
          <TextField
            label="Work email"
            type="email"
            autoComplete="username"
            placeholder="you@acme.io"
            name="email"
          />
          <TextField
            label="Password"
            type="password"
            autoComplete="new-password"
            placeholder="Create a strong password"
            name="password"
            hint="Min 12 chars"
          />
          <Button type="submit" className="mt-1.5">
            Create your gateway
          </Button>
        </Form>

        <p className="flex flex-wrap gap-x-1 text-base text-muted-soft">
          <span>By creating an account, you agree to our</span>
          <a href="/terms" className="font-medium text-primary">
            Terms
          </a>
          <span>and</span>
          <a href="/privacy" className="font-medium text-primary">
            Privacy
          </a>
          <span>Notice.</span>
        </p>
      </div>

      <div className="flex flex-col gap-4">
        <span className="h-px bg-hairline" aria-hidden="true" />
        <div className="flex items-center gap-1.5 text-base">
          <span className="text-muted-soft">Need help?</span>
          <a href="/support" className="font-medium text-primary">
            Contact support
          </a>
        </div>
      </div>
    </section>
  );
}
