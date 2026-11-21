/**
 * The Studio's interface module (Architecture decision 0101, Decision 2401). Every screen builds from what this file exports;
 * nothing outside `src/ui` imports the Astryx design system, so a change of its interface is absorbed
 * here. The names are the Studio's own vocabulary.
 */
export { AnnouncerProvider, useAnnounce } from './Announcer';
export { Banner } from './Banner';
export { Button } from './Button';
export { CheckboxGroup } from './CheckboxGroup';
export type { ButtonProps } from './Button';
export { Actions, Dialog } from './Dialog';
export { ErrorState } from './ErrorState';
export { Field } from './Field';
export { FocusHeading } from './FocusHeading';
export { Checkbox, RadioGroup, Select, TextArea, TextInput } from './inputs';
export type { SelectOption } from './inputs';
export { Disclosure, Divider, Facts, Loading, Page, PageHeader, Panel, Row, Stack } from './layout';
export { Link } from './Link';
export { Progress } from './Progress';
export { UiProvider } from './provider';
export { themeColor } from './theme/roles';
export type { ColorRole } from './theme/roles';
export { Frame } from './shell';
export type { NavEntry } from './shell';
export { Badge, StatusBadge } from './StatusBadge';
export type { Tone } from './StatusBadge';
export { ColumnHeader, RowHeader, Table, TableBody, TableCell, TableHead, TableRow } from './table';
export { Tabs } from './Tabs';
export type { TabSpec } from './Tabs';
export { Code, Empty, Heading, Hint, Text, VisuallyHidden } from './text';
export { useMediaQuery } from './useMediaQuery';
export { WindowedTable } from './WindowedTable';
