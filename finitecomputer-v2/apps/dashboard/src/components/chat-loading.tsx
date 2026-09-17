"use client";

import { useEffect, useState } from "react";

const CHAT_LOADING_PHRASES = [
  "Thank you for your patience, human.",
  "At your service...",
  "A moment, if you please...",
  "Gathering your conversation...",
  "Fetching your words.",
  "Looking for words...",
  "Your thoughts will be with you shortly...",
  "I live to load...",
  "Securely loading our chat...",
  "One moment, please...",
  "Thanks for waiting...",
] as const;

export function ChatLoading() {
  const [typing, setTyping] = useState({ phrase: "", count: 0 });

  useEffect(() => {
    const phrase = CHAT_LOADING_PHRASES[Math.floor(Math.random() * CHAT_LOADING_PHRASES.length)]!;
    let count = 0;
    let timer = window.setTimeout(tick, 0);
    function tick() {
      count += 1;
      setTyping({ phrase, count });
      if (count < phrase.length) timer = window.setTimeout(tick, 28);
    }
    return () => window.clearTimeout(timer);
  }, []);

  return (
    <div className="finite-chat__loading" role="status" aria-label="Loading chat">
      <svg viewBox="0 0 24 24" className="finite-chat__spin finite-chat__loading-wheel" aria-hidden="true">
        {Array.from({ length: 8 }, (_, index) => (
          <circle key={index} cx="12" cy="3" r="1.8" fill="currentColor"
            opacity={(index + 1) / 8} transform={`rotate(${index * 45} 12 12)`} />
        ))}
      </svg>
      <span className="finite-chat__loading-copy" aria-hidden="true">
        <span className="finite-chat__loading-measure">{typing.phrase}</span>
        <span className="finite-chat__loading-typed">{typing.phrase.slice(0, typing.count)}</span>
      </span>
    </div>
  );
}
