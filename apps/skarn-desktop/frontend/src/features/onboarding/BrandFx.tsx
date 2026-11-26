// Selora IDE — Phase Z5b brand surfaces with 3D Gyroscopic & Cosmic Visuals.
//
// The brand mark utilizes high-performance, hardware-accelerated CSS 3D transforms
// (perspective, preserve-3d, gyroscopic orbital rings, celestial planetary sphere
// gradients, and specular reflections). Zero WebGL crashes on WebView2.
import type { ReactNode } from 'react';

export function BrandMark({ className = '' }: { className?: string }): ReactNode {
  return (
    <span className={`onboarding-brand-mark ${className}`.trim()} aria-hidden="true">
      <span className="onboarding-brand-rings">
        <span className="onboarding-brand-ring onboarding-ring-equator" />
        <span className="onboarding-brand-ring onboarding-ring-polar" />
        <span className="onboarding-brand-shadow" />
      </span>
      <span className="onboarding-brand-core" />
      <span className="onboarding-brand-satellite" />
    </span>
  );
}

function HeroFallback({ step = 0 }: { step?: number }) {
  return (
    <div className="onboarding-hero-fallback" data-visual="fallback" data-step={step} aria-hidden="true">
      <div className="onboarding-hero-stage">
        <span className="onboarding-hero-grid" />
        <div className="onboarding-hero-ambient" />
        <div className="onboarding-hero-centerpiece">
          <BrandMark className="onboarding-hero-fallback-mark" />
          <span className="onboarding-hero-pulse-ring" />
        </div>
      </div>
      <span className="onboarding-hero-fallback-label">
        {step === 0 && 'Cosmic Workbench'}
        {step === 1 && 'Connected Tree & Editor'}
        {step === 2 && 'Autonomous Agent Core'}
      </span>
      <span className="onboarding-hero-sub">
        {step === 0 && 'Intelligent · Ultra-Fast · Safe'}
        {step === 1 && 'Zero-lag file search & editing'}
        {step === 2 && 'Approval-gated agent execution'}
      </span>
    </div>
  );
}

export function OnboardingHero({ step = 0 }: { step?: number }) {
  return <HeroFallback step={step} />;
}

export function MetalBrand({ size = 'about' }: { size?: 'about' | 'onboarding' }) {
  return <BrandMark className={`onboarding-metal-fallback onboarding-metal-${size}`} />;
}
