import { Form } from 'react-aria-components';
import { Badge } from '../components/Badge';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Divider } from '../components/Divider';
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
    <aside className="relative flex flex-1 flex-col overflow-hidden bg-surface-dark px-10 py-14 text-on-dark lg:px-16 lg:py-14">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-[-20%] -top-[10%] h-[60%]"
        style={{
          background:
            'radial-gradient(60% 100% at 30% 0%, rgba(156, 90, 44, 0.22) 0%, transparent 65%)',
        }}
      />
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-[-20%] -bottom-[10%] h-[45%]"
        style={{
          background:
            'radial-gradient(50% 80% at 70% 100%, rgba(93, 184, 166, 0.10) 0%, transparent 60%)',
        }}
      />

      <header className="relative z-10 flex items-center gap-3">
        <Wordmark tone="on-dark" />
        <Badge tone="on-dark" className="ml-auto">
          <span className="size-1.5 rounded-full bg-success" aria-hidden="true" />
          all gateways operational
        </Badge>
      </header>

      <section className="relative z-10 mt-20 mb-10 max-w-[540px]">
        <p className="text-xs font-semibold uppercase tracking-wider text-on-dark-soft">
          The gateway for AI software delivery
        </p>
        <h1 className="mt-4 font-serif text-h1 font-medium leading-heading tracking-tighter text-on-dark lg:text-hero">
          Coordinate the work.
          <br />
          Prove the work.
          <br />
          <em className="not-italic text-accent-amber">Ship the work.</em>
        </h1>
        <p className="mt-6 max-w-[480px] text-lg leading-body text-on-dark-soft">
          One front door for every agent and channel.{' '}
          <b className="font-medium text-on-dark">Codex, Claude Code, OpenHands, Symphony</b> in one
          queue — with the gates, the audit trail, and the keyboard discipline operators actually
          want.
        </p>
      </section>

      <Card
        surface="dark"
        elevation="dark"
        padded={false}
        className="relative z-10 mt-3 max-w-[520px] p-5"
      >
        <div className="flex items-center gap-1.5 border-b border-on-dark-soft/15 pb-3">
          <span className="size-2.5 rounded-full bg-[#3d3935]" aria-hidden="true" />
          <span className="size-2.5 rounded-full bg-[#3d3935]" aria-hidden="true" />
          <span className="size-2.5 rounded-full bg-[#3d3935]" aria-hidden="true" />
          <span className="ml-2 font-mono text-xs text-on-dark-soft">
            sanchoris@acme-platform · zsh
          </span>
        </div>
        <div className="mt-3 space-y-1 font-mono text-sm leading-relaxed text-on-dark-soft">
          <div>
            <span className="font-bold text-accent-teal">san&rsaquo;</span>{' '}
            <span className="text-on-dark">queue list</span>{' '}
            <span className="text-muted-soft">--priority p0</span>
          </div>
          <div className="my-1 ml-1 border-l-2 border-on-dark-soft/15 pl-3">
            <div>
              <span className="text-accent-teal">CON-1247</span> Add OAuth login for vendor portal
            </div>
            <div>
              <span className="text-accent-teal">CON-1249</span> Webhook sig mismatch /api/stripe
            </div>
          </div>
          <div>
            <span className="font-bold text-accent-teal">san&rsaquo;</span>{' '}
            <span className="text-on-dark">run start CON-1248</span>
          </div>
          <div className="my-1 ml-1 border-l-2 border-on-dark-soft/15 pl-3">
            <span className="text-success">✓</span>{' '}
            <span className="text-accent-teal">R-9142</span>{' '}
            <span className="text-muted-soft">· workspace ws-growth-c812 · execute ...</span>
          </div>
        </div>
      </Card>

      <footer className="relative z-10 mt-auto flex flex-wrap items-center gap-x-5 gap-y-2 pt-10 text-xs text-on-dark-soft">
        <span>© 2026 Sanchoris</span>
        <span className="h-3.5 w-px bg-on-dark-soft/25" aria-hidden="true" />
        <a href="#status" className="hover:text-on-dark">Status</a>
        <a href="#security" className="hover:text-on-dark">Security</a>
        <a href="#privacy" className="hover:text-on-dark">Privacy</a>
        <a href="#terms" className="hover:text-on-dark">Terms</a>
        <span className="h-3.5 w-px bg-on-dark-soft/25" aria-hidden="true" />
        <span className="font-mono text-muted-soft">v0.3.2 · r-2026.05.24</span>
      </footer>
    </aside>
  );
}

function AuthPanel() {
  return (
    <section className="flex w-full flex-col bg-canvas px-8 py-10 lg:w-[560px] lg:flex-none lg:px-16 lg:py-14">
      <header className="flex items-center gap-3 text-sm text-muted">
        <span>New to Sanchoris?</span>
        <a
          href="#create-account"
          className="rounded-sm border border-hairline px-3 py-1.5 text-sm font-medium text-ink hover:bg-surface-soft"
        >
          Create account
        </a>
        <a href="/" className="ml-auto text-ink hover:text-primary">
          ← Back to home
        </a>
      </header>

      <div className="my-auto w-full max-w-[420px] py-12">
        <p className="text-xs font-semibold uppercase tracking-wider text-muted">Sign in</p>
        <h1 className="mt-3 font-serif text-h1 font-medium leading-heading tracking-tighter text-ink">
          Welcome back
          <br />
          to the gateway.
        </h1>
        <p className="mt-3 mb-9 text-md leading-body text-muted">
          Use your organization SSO or continue with email. Need a workspace?{' '}
          <a href="#workspace" className="font-medium text-primary hover:text-primary-active">
            Get one in 2 minutes →
          </a>
        </p>

        <Card surface="cream-strong" padded={false} className="mb-5 flex items-center gap-3 p-3">
          <span
            aria-hidden="true"
            className="inline-flex size-7 items-center justify-center rounded-sm bg-surface-dark font-mono text-xs font-bold text-on-dark"
          >
            A
          </span>
          <span className="text-sm text-body">
            Detected workspace <b className="font-semibold text-ink">acme-org</b> from your network.
            SAML SSO is enforced for this org.
          </span>
          <button
            type="button"
            aria-label="Dismiss workspace detection"
            className="ml-auto text-sm text-muted-soft hover:text-ink"
          >
            ×
          </button>
        </Card>

        <div className="mb-6 flex flex-col gap-2.5">
          <SSOButton
            variant="primary"
            iconLabel="SS"
            iconTone="dark"
            badge="required"
            label={
              <>
                Continue with <b className="font-semibold">acme-org SAML</b>
              </>
            }
          />
          <SSOButton iconLabel="G" iconTone="dark" meta="org members only" label="Continue with GitHub" />
          <SSOButton
            iconLabel="Gx"
            iconTone="primary"
            meta="@acme.io"
            label="Continue with Google Workspace"
          />
          <SSOButton iconLabel="Az" iconTone="teal" meta="Azure AD" label="Continue with Microsoft Entra" />
        </div>

        <Divider label="or use email" className="my-5" />

        <Form
          className="flex flex-col gap-3"
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
            autoComplete="current-password"
            placeholder="••••••••••"
            name="password"
            rightSlot={<a href="#forgot">Forgot?</a>}
            description={
              <span className="inline-flex items-center gap-1.5">
                <kbd className="rounded-xs border border-hairline border-b-2 bg-surface-soft px-1.5 py-0.5 font-mono text-[10px] text-muted">⇧</kbd>
                <kbd className="rounded-xs border border-hairline border-b-2 bg-surface-soft px-1.5 py-0.5 font-mono text-[10px] text-muted">⏎</kbd>
                to submit · we use Argon2id, never store plaintext
              </span>
            }
          />
          <Button type="submit" className="mt-1.5">
            Sign in to gateway
            <kbd className="rounded-xs bg-white/15 px-1.5 py-0.5 font-mono text-xs">⏎</kbd>
          </Button>
        </Form>

        <Card surface="soft" padded={false} className="mt-4 flex items-center gap-3 p-3.5">
          <span
            aria-hidden="true"
            className="inline-flex size-8 shrink-0 items-center justify-center rounded-md border border-hairline bg-canvas text-primary"
          >
            ✉
          </span>
          <span className="text-sm leading-snug text-body">
            <b className="font-semibold text-ink">Or get a magic link.</b> We'll email you a
            one-time sign-in URL — no password needed.
          </span>
          <a
            href="#magic"
            className="ml-auto text-sm font-medium text-primary hover:text-primary-active"
          >
            Send link →
          </a>
        </Card>

        <ul className="mt-7 flex flex-wrap gap-x-4 gap-y-2 border-t border-hairline-soft pt-5 text-xs text-muted-soft">
          {['SOC 2 Type II', 'SAML / SCIM', 'WebAuthn / Passkeys', 'Self-host available'].map(
            (item) => (
              <li key={item} className="inline-flex items-center gap-1.5">
                <span className="font-semibold text-success">✓</span>
                {item}
              </li>
            ),
          )}
        </ul>
      </div>

      <footer className="mt-auto flex items-center justify-between gap-4 border-t border-hairline-soft pt-6 text-sm text-muted-soft">
        <span>
          Need help?{' '}
          <a href="mailto:support@sanchoris.dev" className="text-ink hover:text-primary">
            support@sanchoris.dev
          </a>
        </span>
        <span className="flex gap-3">
          <a href="#en" className="hover:text-ink">English</a>
          <a href="#ja" className="hover:text-ink">日本語</a>
        </span>
      </footer>
    </section>
  );
}
