// Scoped read-only macOS Accessibility API smoke test for a running SliceBridge.app.
// Usage: swift native/tests/macos-native-a11y.swift --lang he|en --scope main|settings|picker
// This checks OS-accessible roles/names/focus; it does NOT test VoiceOver speech
// or certify WCAG conformance. No app settings, windows or user input are changed.
import Cocoa
import ApplicationServices

let args=CommandLine.arguments
func argument(_ name:String)->String? {
  guard let index=args.firstIndex(of:name),index+1<args.count else {return nil}
  return args[index+1]
}
guard let lang=argument("--lang"),["he","en"].contains(lang),
      let scope=argument("--scope"),["main","settings","picker"].contains(scope) else {
  fputs("Usage: swift macos-native-a11y.swift --lang he|en --scope main|settings|picker\n",stderr)
  exit(2)
}
guard AXIsProcessTrusted() else {
  print("NATIVE_AX_BLOCKED: macOS Accessibility permission not granted to this runner")
  exit(3)
}
guard let app=NSRunningApplication.runningApplications(withBundleIdentifier:"com.wails.slicebridge").first else {
  print("NATIVE_AX_BLOCKED: no running SliceBridge.app")
  exit(3)
}
struct Item {let role:String;let name:String;let focus:String}
func attribute(_ element:AXUIElement,_ key:String)->String {
  var value:CFTypeRef?
  guard AXUIElementCopyAttributeValue(element,key as CFString,&value) == .success else{return ""}
  return String(describing:value!)
}
func children(_ element:AXUIElement)->[AXUIElement] {
  var value:CFTypeRef?
  guard AXUIElementCopyAttributeValue(element,"AXChildren" as CFString,&value) == .success else {return []}
  return value as? [AXUIElement] ?? []
}
var items=[Item]()
var visited=0
func collect(_ element:AXUIElement,_ depth:Int=0) {
  if depth>16 || visited>650{return}
  let role=attribute(element,"AXRole")
  if role=="AXMenuBar"{return}
  let name=attribute(element,"AXTitle")
  let description=attribute(element,"AXDescription")
  let label=name.isEmpty ? description : name
  let focused=attribute(element,"AXFocused")
  items.append(Item(role:role,name:label,focus:focused))
  visited+=1
  for (i,child) in children(element).enumerated(){
    // The 603-model picker has a very large AX tree; its first entries suffice
    // for container-label verification. Do not scan every model.
    if i>=8 && description=="Catalogue models" {break}
    if i>=8 && description=="דגמי מדפסות" {break}
    collect(child,depth+1)
  }
}
collect(AXUIElementCreateApplication(app.processIdentifier))
func has(_ role:String,_ name:String,_ mustFocus:Bool=false)->Bool {
  items.contains{$0.role==role && $0.name==name && (!mustFocus || $0.focus=="1")}
}
let he=lang=="he"
var checks=[(String,Bool)]()
checks.append(("native_web_area",items.contains{$0.role=="AXWebArea"}))
switch scope {
case "main":
  checks.append(("localized_dropzone_button",has("AXButton",he ? "הוסף קבצים":"Add files")))
  checks.append(("localized_settings_button",has("AXButton",he ? "הגדרות":"Settings")))
case "settings":
  checks.append(("settings_dialog_heading",has("AXHeading",he ? "הגדרות מדפסות":"Printer settings")))
  checks.append(("localized_close_button",has("AXButton",he ? "סגירה":"Close")))
  checks.append(("focus_inside_dialog",items.contains{$0.role=="AXButton" && $0.focus=="1"}))
case "picker":
  checks.append(("localized_catalog_group",has("AXGroup",he ? "דגמי מדפסות":"Catalogue models")))
  checks.append(("localized_manufacturer_control",has("AXPopUpButton",he ? "בחירת יצרן":"Select manufacturer")))
  checks.append(("search_field",has("AXTextField",he ? "או חפש דגם":"Or search a model")))
default: break
}
let passed=checks.allSatisfy{$0.1}
let report:[String:Any]=[
  "status":passed ? "NATIVE_AX_PASS":"NATIVE_AX_FAIL",
  "scope":scope,"lang":lang,"pid":Int(app.processIdentifier),
  "appPath":app.bundleURL?.path ?? "unknown",
  "nodesVisited":visited,
  "speechReaderTested":false,
  "checks":checks.map{["name":$0.0,"passed":$0.1] as [String:Any]}
]
if let data=try? JSONSerialization.data(withJSONObject:report,options:[.sortedKeys]),
   let value=String(data:data,encoding:.utf8) {print(value)}
exit(passed ? 0:1)
