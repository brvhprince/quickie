import { mount } from "svelte";
import WebApp from "./WebApp.svelte";
import "../src/styles/base.css";
import "./web.css";

export default mount(WebApp, { target: document.getElementById("app")! });
