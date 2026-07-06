"use client";

import {
  BotIcon,
  ChevronDownIcon,
  MailIcon,
  MoreHorizontalIcon,
  PlusIcon,
  SearchIcon,
  SettingsIcon,
} from "lucide-react";

import { Avatar, AvatarFallback, AvatarGroup, AvatarGroupCount, AvatarImage } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
  InputGroupText,
} from "@/components/ui/input-group";
import { Kbd } from "@/components/ui/kbd";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { TYPE_SCALE } from "@/lib/typography";
import { statusBadgeToneClass, type StatusBadgeState } from "@/lib/status-badge-tone";

import { StickerBlock, StickerRow } from "./sticker-sheet-section";

const STATUS_SAMPLES: StatusBadgeState[] = ["pending", "in_progress", "complete", "blocked"];
const BUTTON_VARIANTS = ["default", "outline", "secondary", "ghost", "destructive", "link"] as const;
const BUTTON_SIZES = ["default", "xs", "sm", "lg", "icon", "icon-xs", "icon-sm", "icon-lg"] as const;
const BADGE_VARIANTS = ["default", "secondary", "destructive", "outline", "ghost", "link"] as const;

export function StickerSheetPrimitives() {
  return (
    <>
      <StickerBlock title="Typography">
        <div className="overflow-x-auto rounded-xl border border-border">
          <table className="w-full min-w-[720px] border-collapse text-left">
            <thead>
              <tr className="border-b border-border bg-muted/40">
                <th className="type-caption px-4 py-3 font-medium text-muted-foreground">Sample</th>
                <th className="type-caption px-4 py-3 font-medium text-muted-foreground">Class</th>
                <th className="type-caption px-4 py-3 font-medium text-muted-foreground">Size</th>
                <th className="type-caption px-4 py-3 font-medium text-muted-foreground">Line</th>
                <th className="type-caption px-4 py-3 font-medium text-muted-foreground">Weight</th>
                <th className="type-caption px-4 py-3 font-medium text-muted-foreground">Tracking</th>
                <th className="type-caption px-4 py-3 font-medium text-muted-foreground">Use</th>
              </tr>
            </thead>
            <tbody>
              {TYPE_SCALE.map((step) => {
                const sampleLabel = step.className.startsWith("type-mono")
                  ? `JetBrains Mono - ${step.name}`
                  : `Funnel Sans - ${step.name}`;
                return (
                  <tr key={step.className} className="border-b border-border/60 last:border-0">
                    <td className="px-4 py-4">
                      <span className={step.className}>{sampleLabel}</span>
                    </td>
                    <td className="px-4 py-4">
                      <code className="type-mono-sm text-muted-foreground">{step.className}</code>
                    </td>
                    <td className="type-body-sm px-4 py-4 text-muted-foreground">{step.size}</td>
                    <td className="type-body-sm px-4 py-4 text-muted-foreground">{step.lineHeight}</td>
                    <td className="type-body-sm px-4 py-4 text-muted-foreground">{step.weight}</td>
                    <td className="type-body-sm px-4 py-4 text-muted-foreground">{step.tracking}</td>
                    <td className="type-body-sm px-4 py-4 text-muted-foreground">{step.use}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      </StickerBlock>

      <StickerBlock title="Buttons - variants">
        <StickerRow>
          {BUTTON_VARIANTS.map((variant) => (
            <Button key={variant} variant={variant}>
              {variant}
            </Button>
          ))}
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Buttons - sizes">
        <StickerRow>
          {BUTTON_SIZES.map((size) => (
            <Button key={size} size={size} variant={size.startsWith("icon") ? "outline" : "default"}>
              {size.startsWith("icon") ? <SettingsIcon /> : size}
            </Button>
          ))}
        </StickerRow>
        <StickerRow className="mt-3">
          <Button disabled>Disabled</Button>
          <Button variant="outline" disabled>
            Disabled outline
          </Button>
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Badges">
        <StickerRow>
          {BADGE_VARIANTS.map((variant) => (
            <Badge key={variant} variant={variant}>
              {variant}
            </Badge>
          ))}
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Status badges">
        <StickerRow>
          {STATUS_SAMPLES.map((status) => (
            <span key={status} className={statusBadgeToneClass(status)}>
              {status.replace("_", " ")}
            </span>
          ))}
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Form controls">
        <div className="grid max-w-xl gap-4">
          <div className="grid gap-2">
            <Label htmlFor="sticker-input">Label</Label>
            <Input id="sticker-input" placeholder="Placeholder text" />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="sticker-invalid">Invalid input</Label>
            <Input id="sticker-invalid" aria-invalid defaultValue="bad@value" />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="sticker-textarea">Textarea</Label>
            <Textarea id="sticker-textarea" placeholder="Multi-line input" rows={3} />
          </div>
          <div className="grid gap-2">
            <Label htmlFor="sticker-select">Select</Label>
            <Select defaultValue="alpha">
              <SelectTrigger id="sticker-select" className="w-full max-w-xs">
                <SelectValue placeholder="Pick one" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="alpha">Alpha machine</SelectItem>
                <SelectItem value="beta">Beta machine</SelectItem>
                <SelectItem value="gamma">Gamma machine</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <InputGroup className="max-w-md">
            <InputGroupAddon>
              <SearchIcon />
            </InputGroupAddon>
            <InputGroupInput placeholder="Search machines..." />
            <InputGroupAddon align="inline-end">
              <InputGroupButton aria-label="More">
                <MoreHorizontalIcon />
              </InputGroupButton>
            </InputGroupAddon>
          </InputGroup>
          <InputGroup className="max-w-md">
            <InputGroupAddon>
              <InputGroupText>https://</InputGroupText>
            </InputGroupAddon>
            <InputGroupInput placeholder="site.finite.vip" />
          </InputGroup>
        </div>
      </StickerBlock>

      <StickerBlock title="Keyboard">
        <StickerRow>
          <Kbd>Cmd</Kbd>
          <Kbd>K</Kbd>
          <span className="text-sm text-muted-foreground">+</span>
          <Kbd>Shift</Kbd>
          <Kbd>P</Kbd>
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Avatars">
        <StickerRow>
          <Avatar>
            <AvatarImage src="https://api.dicebear.com/9.x/shapes/svg?seed=finite" alt="User" />
            <AvatarFallback>FC</AvatarFallback>
          </Avatar>
          <Avatar size="sm">
            <AvatarFallback>SM</AvatarFallback>
          </Avatar>
          <Avatar size="lg">
            <AvatarFallback>LG</AvatarFallback>
          </Avatar>
          <AvatarGroup>
            <Avatar>
              <AvatarFallback>A</AvatarFallback>
            </Avatar>
            <Avatar>
              <AvatarFallback>B</AvatarFallback>
            </Avatar>
            <AvatarGroupCount>+3</AvatarGroupCount>
          </AvatarGroup>
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Cards">
        <div className="grid max-w-2xl gap-4 md:grid-cols-2">
          <Card>
            <CardHeader>
              <CardTitle>Default card</CardTitle>
              <CardDescription>Card description with muted foreground.</CardDescription>
              <CardAction>
                <Button size="sm" variant="outline">
                  Action
                </Button>
              </CardAction>
            </CardHeader>
            <CardContent>
              <p className="text-sm">Card body content goes here.</p>
            </CardContent>
            <CardFooter>
              <Button size="sm">Save</Button>
            </CardFooter>
          </Card>
          <Card size="sm">
            <CardHeader>
              <CardTitle>Small card</CardTitle>
              <CardDescription>Compact density variant.</CardDescription>
            </CardHeader>
            <CardContent>
              <Skeleton className="h-4 w-full" />
              <Skeleton className="mt-2 h-4 w-3/4" />
            </CardContent>
          </Card>
        </div>
      </StickerBlock>

      <StickerBlock title="Tabs">
        <Tabs defaultValue="overview" className="max-w-md">
          <TabsList>
            <TabsTrigger value="overview">Overview</TabsTrigger>
            <TabsTrigger value="sites">Sites</TabsTrigger>
            <TabsTrigger value="secrets">Secrets</TabsTrigger>
          </TabsList>
          <TabsContent value="overview" className="text-sm text-muted-foreground">
            Overview tab panel content.
          </TabsContent>
          <TabsContent value="sites" className="text-sm text-muted-foreground">
            Sites tab panel content.
          </TabsContent>
          <TabsContent value="secrets" className="text-sm text-muted-foreground">
            Secrets tab panel content.
          </TabsContent>
        </Tabs>
      </StickerBlock>

      <StickerBlock title="Table">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Machine</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Region</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow>
              <TableCell>paul-finite</TableCell>
              <TableCell>
                <span className={statusBadgeToneClass("complete")}>complete</span>
              </TableCell>
              <TableCell>box1</TableCell>
            </TableRow>
            <TableRow>
              <TableCell>demo-agent</TableCell>
              <TableCell>
                <span className={statusBadgeToneClass("in_progress")}>in progress</span>
              </TableCell>
              <TableCell>local</TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </StickerBlock>

      <StickerBlock title="Overlays">
        <StickerRow>
          <Dialog>
            <DialogTrigger asChild>
              <Button variant="outline">Dialog</Button>
            </DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>Dialog title</DialogTitle>
                <DialogDescription>Modal content using shadcn Dialog.</DialogDescription>
              </DialogHeader>
              <p className="text-sm">Body copy inside the dialog.</p>
              <DialogFooter>
                <Button variant="outline">Cancel</Button>
                <Button>Confirm</Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>

          <Sheet>
            <SheetTrigger asChild>
              <Button variant="outline">Sheet</Button>
            </SheetTrigger>
            <SheetContent>
              <SheetHeader>
                <SheetTitle>Sheet title</SheetTitle>
                <SheetDescription>Slide-over panel from the right.</SheetDescription>
              </SheetHeader>
              <p className="px-4 text-sm text-muted-foreground">Sheet body content.</p>
            </SheetContent>
          </Sheet>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline">
                Menu
                <ChevronDownIcon />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              <DropdownMenuLabel>Actions</DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuItem>
                <MailIcon />
                Email
              </DropdownMenuItem>
              <DropdownMenuItem>
                <BotIcon />
                Agent
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>

          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="outline">Tooltip</Button>
            </TooltipTrigger>
            <TooltipContent>Helpful hint text</TooltipContent>
          </Tooltip>
        </StickerRow>
      </StickerBlock>

      <StickerBlock title="Separator">
        <div className="max-w-md space-y-4">
          <p className="text-sm">Above separator</p>
          <Separator />
          <p className="text-sm">Below separator</p>
        </div>
      </StickerBlock>

      <StickerBlock title="Empty state">
        <div className="flex max-w-md items-center gap-3 rounded-xl border border-border bg-card p-4">
          <div className="flex size-10 items-center justify-center rounded-full bg-muted">
            <PlusIcon className="size-4" />
          </div>
          <div>
            <p className="type-title-3">New machine</p>
            <p className="type-body-sm text-muted-foreground">Compact system copy with the same scale.</p>
          </div>
        </div>
      </StickerBlock>
    </>
  );
}
