import React, { useEffect, useState } from "react";
import { useFileUpload } from "./fileUploadContext";

// const output = [[]];
// let lineNum = 0;
// let focus = false;
// let firstRun = true;
//
// function caretToggle() {
//   if (!focus) {
//     return;
//   }
//   const caret = document.getElementsByClassName("term-caret")[0];
//   if (caret.classList.contains("blink")) {
//     caret.classList.remove("blink");
//   } else {
//     caret.classList.add("blink");
//   }
// }
//
// function focusTerminal() {
//   const terminalDiv = document.getElementById("terminal-text");
//   terminalDiv.classList.add("focus");
//   focus = true;
// }
//
// function unFocusTerminal() {
//   const terminalDiv = document.getElementById("terminal-text");
//   const caret = document.getElementsByClassName("term-caret")[0];
//   terminalDiv.classList.remove("focus");
//   caret.classList.add("blink");
//   focus = false;
// }

// function print(outputStr) {
//   const terminalText = document.querySelector("#terminal-text");
//   const result = terminalText.childNodes[0];
//   result.nodeValue += "\n";
//   lineNum += 1;
//   output.push([]);
//   for (let i = 0; i < outputStr.length; i += 1) {
//     const character = outputStr[i];
//     output[lineNum].push(character);
//     result.nodeValue += character;
//   }
//   // add a new line after print
//   result.nodeValue += "\nroot$ ";
//   lineNum += 1;
//   output.push([]);
// }

// function unwind(num) {
//   const terminalText = document.querySelector("#terminal-text");
//   const result = terminalText.childNodes[0];
//   let newNum = num;
//   // dont let unwind if nothing to unwind
//   if (result.nodeValue.length <= 6) {
//     result.nodeValue = "root$ ";
//     return;
//   }
//   // delete entire line if unwind is large enough
//   if (num > output[lineNum].length) {
//     newNum -= output[lineNum].length;
//     result.nodeValue = result.nodeValue.substring(
//       0,
//       result.nodeValue.length - output[lineNum].length - 7
//     );
//     // do not remove last entry in output
//     if (output.length > 1) {
//       output.pop();
//       lineNum -= 1;
//     } else {
//       output[lineNum] = [];
//     }
//     // if there is more characters to delete continue deletion
//     if (newNum >= 0) {
//       unwind(newNum);
//     }
//   } else if (output[lineNum].length > 0) {
//     result.nodeValue = result.nodeValue.substring(
//       0,
//       result.nodeValue.length - newNum
//     );
//     output[lineNum] = output[lineNum].slice(0, -newNum);
//   }
// }

// function logKey(e) {
//   if (!focus) {
//     return;
//   }
//   // prevent scrolling with spacebar
//   if (e.keyCode === 32 && e.target === document.body) {
//     e.preventDefault();
//   }
//   const terminalText = document.querySelector("#terminal-text");
//   const character = `${String.fromCharCode(e.keyCode)}`.toLowerCase();
//   const result = terminalText.childNodes[0];
//   if (e.keyCode === 13) {
//     // enter pressed
//     let command = output[lineNum].join("");
//     command = command.replace(/\s/g, ""); // remove spaces
//     if (command === "unwind") {
//       unwind(5 + 6);
//     } else if (command === "print") {
//       print("test\ntest2");
//     } else {
//       lineNum += 1;
//       output.push([]);
//       result.nodeValue += "\nroot$ ";
//     }
//   } else if (e.keyCode === 8) {
//     // dont delete if there is nothing to delete
//     if (output[lineNum].length > 0) {
//       output[lineNum].pop();
//       result.nodeValue = result.nodeValue.substring(
//         0,
//         result.nodeValue.length - 1
//       );
//     }
//   } else {
//     output[lineNum].push(character);
//     result.nodeValue += character;
//   }
// }

export default function Terminal() {
  const { addListener } = useFileUpload();
  const [content, setContent] = useState("");

  useEffect(() => {
    // do this setup only once
    // if (firstRun) {
    //   document.addEventListener("keydown", logKey);
    //   const terminalDiv = document.getElementById("terminal-div");
    //   const editorDiv = document.getElementById("editor-div");
    //   terminalDiv.addEventListener("click", focusTerminal);
    //   editorDiv.addEventListener("click", unFocusTerminal);
    //   setInterval(caretToggle, 500);
    //   firstRun = false;
    // }

    addListener("Stdout", (send, resp, data) => {
      setContent((c) => c + data);
    });

    addListener("Compiled", (send, _resp, _data) => {
      send("RunOp", undefined);
      setContent("");
    });

    addListener("Status", (send, _resp, _data) => {
      send("RunOp", undefined);
    });

    addListener("CompileError", (send, _resp, data) => {
      setContent(data.rendered);
    });
  }, []);

  return (
    <div>
      <div
        id="terminal-title"
        className="h-10 text-white bg-gray-800 py-1 px-6 w-full"
      >
        <div>Terminal</div>
      </div>
      <div
        className="h-screen w-full p-2"
        style={{ backgroundColor: "#1E1E1E" }}
      >
        <pre id="terminal-text">{content}</pre>
      </div>
    </div>
  );
}
