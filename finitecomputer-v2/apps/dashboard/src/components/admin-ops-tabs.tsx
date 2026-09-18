"use client";

import { createContext, useContext, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import styles from "@/styles/admin-ops.module.css";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

import { AdminEmailChangeForm } from "@/components/admin-email-change-form";

const ToolbarTarget = createContext<HTMLDivElement | null>(null);

export function AdminOpsTabs({ children }: { children: ReactNode }) {
  const [section, setSection] = useState("users");
  const [toolbar, setToolbar] = useState<HTMLDivElement | null>(null);
  return (
    <ToolbarTarget.Provider value={toolbar}>
      <Tabs value={section} onValueChange={setSection} className="gap-4">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <TabsList aria-label="Admin sections" className={styles.sectionTabs} style={{ height: 40 }}>
            <TabsTrigger value="users">Users</TabsTrigger>
            <TabsTrigger value="invites">Invites</TabsTrigger>
            <TabsTrigger value="finite-private">Finite Private</TabsTrigger>
            <TabsTrigger value="email">Email</TabsTrigger>
          </TabsList>
          <div ref={setToolbar} hidden={section !== "users"} className="max-w-full" />
        </div>
        {children}
        <TabsContent value="email">
          <AdminEmailChangeForm />
        </TabsContent>
      </Tabs>
    </ToolbarTarget.Provider>
  );
}

export function AdminUsersToolbar({ children }: { children: ReactNode }) {
  const target = useContext(ToolbarTarget);
  return target ? createPortal(children, target) : null;
}
