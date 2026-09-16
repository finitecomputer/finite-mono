"use client";

import { useEffect, useRef, useState } from "react";
import { ArrowLeftIcon, ArrowRightIcon, CheckIcon, LoaderCircleIcon, LockKeyholeIcon } from "lucide-react";
import { LogoIcon } from "@/components/logo";
import { CoreAgentReadyPanel } from "@/components/core-agent-ready-panel";
import { FiniteBrand } from "@/components/finite-brand";
import { FiniteLoader } from "@/components/finite-loader";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import "@/styles/ocean-shell.css";

type Step = "code" | "billing" | "name" | "launch" | "ready";
type CodeStatus = "idle" | "checking" | "valid" | "invalid";
const steps: Step[] = ["code", "billing", "name", "launch", "ready"];
const launchSteps = ["Creating your agent", "Preparing their workspace", "Connecting to chat"];
const headingClass = "font-sans text-3xl leading-tight font-medium tracking-[-0.02em] sm:text-5xl";

// This entire flow is a development-only design preview. Nothing here redeems
// a launch code, takes payment, or creates an agent. Real validation and billing
// remain owned by the existing production flow until the design is approved.
export function OnboardingPreview() {
  const [step, setStep] = useState<Step>("code");
  const [code, setCode] = useState("");
  const [codeStatus, setCodeStatus] = useState<CodeStatus>("idle");
  const [freeTrial, setFreeTrial] = useState(false);
  const [name, setName] = useState("");
  const nameIsValid = /^[a-zA-Z0-9]+$/.test(name);
  const nameError = name.length > 0 && !nameIsValid
    ? "Use letters and numbers only. No spaces or symbols."
    : null;
  const [launchIndex, setLaunchIndex] = useState(0);
  const heading = useRef<HTMLHeadingElement>(null);

  useEffect(() => { heading.current?.focus(); }, [step]);

  useEffect(() => {
    if (step !== "code" || !code.trim()) return;
    const check = window.setTimeout(() => setCodeStatus("checking"), 350);
    const result = window.setTimeout(() => {
      setCodeStatus(code.trim().toUpperCase() === "FINITE-DEMO" ? "valid" : "invalid");
    }, 1150);
    return () => { window.clearTimeout(check); window.clearTimeout(result); };
  }, [code, step]);

  useEffect(() => {
    if (step !== "code" || codeStatus !== "valid") return;
    const timer = window.setTimeout(() => {
      setFreeTrial(true);
      setStep("billing");
    }, 850);
    return () => window.clearTimeout(timer);
  }, [codeStatus, step]);

  useEffect(() => {
    if (step !== "launch") return;
    const timers = [
      window.setTimeout(() => setLaunchIndex(1), 1600),
      window.setTimeout(() => setLaunchIndex(2), 3200),
      window.setTimeout(() => setStep("ready"), 5000),
    ];
    return () => timers.forEach(window.clearTimeout);
  }, [step]);

  function restart() {
    setStep("code"); setCode(""); setCodeStatus("idle");
    setFreeTrial(false); setName(""); setLaunchIndex(0);
  }

  return (
    <div className="ocean-shell">
      <div className="relative flex min-h-dvh flex-col bg-background text-foreground">
        <div className="pointer-events-none absolute inset-0 opacity-70" aria-hidden style={{ background: "radial-gradient(circle at 50% 32%, color-mix(in srgb, var(--accent-blue) 12%, transparent), transparent 38%)" }} />
        <header className="relative z-10 flex items-center justify-between gap-4 px-5 py-5 sm:px-8 sm:py-7">
          <FiniteBrand href="/dev/onboarding" />
          <ol className="grid w-32 grid-cols-5 gap-1 sm:w-40" aria-label="Agent setup progress">
            {steps.map((item, index) => (
              <li key={item} aria-current={step === item ? "step" : undefined} className={`h-0.5 rounded-full ${index <= steps.indexOf(step) ? "bg-foreground" : "bg-border"}`}>
                <span className="sr-only">{item}: {step === item ? "current" : index < steps.indexOf(step) ? "complete" : "upcoming"}</span>
              </li>
            ))}
          </ol>
        </header>
        <main className="relative z-10 mx-auto grid w-full max-w-3xl flex-1 content-center justify-items-center gap-8 px-5 py-12 sm:px-8">
          {step === "code" && (
            <section className="grid w-full justify-items-center gap-7 text-center">
              <div className="grid gap-3">
                <h1 ref={heading} tabIndex={-1} className={headingClass}>Do you have a launch code?</h1>
                <p className="type-body-lg text-muted-foreground">If not, click Continue without a code</p>
              </div>
              <div className="grid w-full max-w-md gap-2 text-left">
                <Label htmlFor="preview-code">Paste code</Label>
                <div className="relative">
                  <Input id="preview-code" className="h-12 pr-12 text-base" placeholder="finite-xxxxxxx" autoComplete="off" spellCheck={false} value={code} aria-invalid={codeStatus === "invalid"} aria-describedby="code-feedback" onChange={(event) => { setCode(event.target.value); setCodeStatus("idle"); }} />
                  <span className="absolute inset-y-0 right-4 flex items-center" aria-hidden>
                    {codeStatus === "checking" && <LoaderCircleIcon className="size-5 animate-spin motion-reduce:animate-none" />}
                    {codeStatus === "valid" && <CheckIcon className="size-5 text-emerald-600" />}
                  </span>
                </div>
                <p id="code-feedback" role="status" className={codeStatus === "idle" ? "sr-only" : `text-sm ${codeStatus === "invalid" ? "text-destructive" : "text-muted-foreground"}`}>
                  {codeStatus === "checking" ? "Checking your code…" : codeStatus === "valid" ? "Code accepted. Your free trial is ready." : codeStatus === "invalid" ? "That code didn’t work. Try another, or continue without one." : ""}
                </p>
              </div>
              <Button size="xl" variant="outline" onClick={() => { setFreeTrial(false); setStep("billing"); }}>Continue without a code <ArrowRightIcon /></Button>
            </section>
          )}
          {step === "billing" && (
            <section className="grid w-full justify-items-center gap-7 text-center">
              <div className="grid gap-3">
                <h1 ref={heading} tabIndex={-1} className={headingClass}>{freeTrial ? "Your free trial is ready." : "Your own Finite Agent."}</h1>
              </div>
              <div className="grid w-full max-w-md gap-4">
                <Card className="gap-0 overflow-hidden rounded-2xl border-border/70 bg-card py-0 text-left shadow-sm">
                  <CardContent className="grid gap-6 p-6 sm:p-8">
                    <div className="flex items-center gap-3">
                      <div className="flex size-11 shrink-0 items-center justify-center rounded-xl border border-border/60 bg-muted/50" aria-hidden>
                        <LogoIcon className="text-2xl" style={{ maskImage: 'url("/finite-onboarding-logo.svg")', WebkitMaskImage: 'url("/finite-onboarding-logo.svg")' }} />
                      </div>
                      <div className="grid gap-0.5">
                        <span className="text-xs text-muted-foreground">Finite Computer</span>
                        <h2 className="text-base font-medium">Hosted Agent</h2>
                      </div>
                    </div>
                    <div className="grid gap-3 border-t border-border/70 pt-6">
                      <div className={`flex flex-wrap items-baseline gap-x-2 gap-y-1 ${freeTrial ? "text-muted-foreground" : "text-foreground"}`}>
                        <span className="text-3xl font-semibold tracking-tight tabular-nums">{freeTrial ? <s>$200</s> : "$200"}</span>
                        <span className="text-sm text-muted-foreground">/ month</span>
                      </div>
                      {freeTrial && <div className="text-3xl font-semibold tracking-tight">Free trial</div>}
                      <p className="text-sm text-muted-foreground">{freeTrial ? "Launch code applied. No payment needed." : "Billed monthly. Cancel anytime."}</p>
                    </div>
                    <div className="grid gap-3">
                      <Button size="xl" className="w-full justify-between" onClick={() => setStep("name")}>Continue <ArrowRightIcon /></Button>
                      {!freeTrial && (
                        <p className="flex items-center justify-center gap-1.5 text-xs text-muted-foreground">
                          <LockKeyholeIcon className="size-3.5" aria-hidden />
                          Secure checkout with Stripe
                        </p>
                      )}
                    </div>
                  </CardContent>
                </Card>
                <p className="px-3 text-xs leading-relaxed text-muted-foreground">{freeTrial ? "Your launch code covers your trial." : "Tax added at checkout where applicable. Setup begins after payment. Manage or cancel your subscription in the billing portal."}</p>
              </div>
              <Button variant="ghost" onClick={() => { setCode(""); setCodeStatus("idle"); setStep("code"); }}><ArrowLeftIcon /> Back</Button>
            </section>
          )}
          {step === "name" && (
            <form className="grid w-full justify-items-center gap-7 text-center" onSubmit={(event) => { event.preventDefault(); if (nameIsValid) { setLaunchIndex(0); setStep("launch"); } }}>
              <div className="grid gap-3">
                <h1 ref={heading} tabIndex={-1} className={headingClass}>Name your agent</h1>
              </div>
              <div className="grid w-full max-w-md gap-2 text-left">
                <Label htmlFor="preview-name">Agent name</Label>
                <Input id="preview-name" className="h-12 text-base" autoComplete="off" value={name} maxLength={80} required aria-invalid={Boolean(nameError)} aria-describedby={nameError ? "preview-name-error" : undefined} onChange={(event) => setName(event.target.value)} />
                {nameError && <p id="preview-name-error" role="status" className="text-sm text-destructive">{nameError}</p>}
              </div>
              <Button size="xl" type="submit" disabled={!nameIsValid}>Continue <ArrowRightIcon /></Button>
              <Button type="button" variant="ghost" onClick={() => setStep("billing")}><ArrowLeftIcon /> Back</Button>
            </form>
          )}
          {step === "launch" && (
            <section className="grid min-h-[28rem] content-center justify-items-center gap-7 text-center" role="status" aria-live="polite">
              <FiniteLoader label="Launching your agent" size={72} variant="center-out" />
              <h1 ref={heading} tabIndex={-1} className={headingClass}>Launching {name.trim()}.</h1>
              <p className="type-body-lg text-muted-foreground">We’re getting everything ready for you.</p>
              <ol className="grid gap-3 text-left text-sm">
                {launchSteps.map((label, index) => <li key={label} className={`flex items-center gap-3 ${index > launchIndex ? "text-muted-foreground/50" : "text-foreground"}`}>
                  {index < launchIndex ? <CheckIcon className="size-4" /> : index === launchIndex ? <LoaderCircleIcon className="size-4 animate-spin motion-reduce:animate-none" /> : <span className="size-4 rounded-full border border-border" />}{label}
                </li>)}
              </ol>
            </section>
          )}
          {step === "ready" && <CoreAgentReadyPanel name={name.trim()} chatHref="/dashboard/machines/runtime_web_design/chat" />}
        </main>
        <footer className="relative z-10 flex flex-wrap items-center justify-center gap-x-3 gap-y-1 px-5 py-4 text-center text-xs text-muted-foreground">
          <span>Design preview · Use FINITE-DEMO · No charges or agents created · Opens sample chat</span>
          <button type="button" className="underline underline-offset-4 hover:text-foreground" onClick={restart}>Start over</button>
        </footer>
      </div>
    </div>
  );
}
