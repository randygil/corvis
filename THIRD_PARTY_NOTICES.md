# Third-party notices

Corvis is original software, published under the MIT license (see `LICENSE`). To be compatible
with the Node.js ecosystem, it replicates the API, behavior, messages and some algorithms of
the projects below, and in some cases it translates their logic to Rust or includes small templates and
data of theirs. This file keeps their copyright notices and licenses, as those licenses require.

Corvis is not affiliated with or endorsed by these projects. The names of the projects and of
their products are trademarks of their respective owners and are used here only to identify them.

## Components

| Project | License | Copyright | What Corvis takes |
|---|---|---|---|
| NestJS (`@nestjs/common`, `@nestjs/core`, `@nestjs/platform-express`, `@nestjs/testing`, `@nestjs/cli` and the official packages `@nestjs/config`, `@nestjs/jwt`, `@nestjs/schedule`, `@nestjs/swagger`, `@nestjs/throttler`, `@nestjs/terminus`, `@nestjs/cache-manager`, `@nestjs/event-emitter`, `@nestjs/serve-static`, `@nestjs/websockets`, `@nestjs/platform-ws`) | MIT | Copyright (c) 2017-present Kamil Mysliwiec <https://kamilmysliwiec.com> | API design (modules, dependency injection, decorators, lifecycle, enhancers), behavior, error and log messages; the Swagger UI HTML/JS templates of `@nestjs/swagger` (`crates/corvis-swagger/src/assets/swagger-ui-*.tpl`). |
| zod | MIT | Copyright (c) 2025 Colin McDonnell | `corvis::z`: API, validation semantics, error messages, format regular expressions and `toJSONSchema`. |
| nestjs-zod | MIT | Copyright (c) Ben Lorantfy <ben@lorantfy.com> | `ZodValidationPipe`, serializer, guard and the cleanup of the OpenAPI document. |
| Prisma (`prisma`, `@prisma/client`, prisma-engines) | Apache-2.0 | Copyright Prisma Data, Inc. | `corvis-orm`: schema language (`schema.prisma`), shape of the client API, query semantics, migration format and `_prisma_migrations` table, error codes and messages. |
| Express | MIT | Copyright (c) 2009-2014 TJ Holowaychuk <tj@vision-media.ca><br>Copyright (c) 2013-2014 Roman Shtylman <shtylman+expressjs@gmail.com><br>Copyright (c) 2014-2015 Douglas Christopher Wilson <doug@somethingdoug.com> | Router semantics, response (`res.*`) semantics and default headers. |
| body-parser | MIT | Copyright (c) 2014 Jonathan Ong <me@jongleberry.com><br>Copyright (c) 2014-2015 Douglas Christopher Wilson <doug@somethingdoug.com> | `http/body_parser.rs`: limits, types and errors. |
| raw-body | MIT | Copyright (c) 2013-2014 Jonathan Ong <me@jongleberry.com><br>Copyright (c) 2014-2022 Douglas Christopher Wilson <doug@somethingdoug.com> | Body reading and its errors. |
| qs | BSD-3-Clause | Copyright (c) 2014, Nathan LaFreniere and other [contributors](https://github.com/ljharb/qs/graphs/contributors) | Parser for `application/x-www-form-urlencoded` and query strings. |
| busboy | MIT | Copyright Brian White. All rights reserved. | Parser for `multipart/form-data` (`multer/busboy.rs`). |
| multer | MIT | Copyright (c) 2014 Hage Yaapa <[http://www.hacksparrow.com](http://www.hacksparrow.com)> | `corvis::multer`: storage, limits and errors. |
| append-field | MIT | Copyright (c) 2015 Linus Unnebäck | Assembly of nested multipart fields. |
| cors | MIT | Copyright (c) 2013 Troy Goode <troygoode@gmail.com> | `router/cors.rs`. |
| cookie | MIT | Copyright (c) 2012-2014 Roman Shtylman <shtylman@gmail.com><br>Copyright (c) 2015 Douglas Christopher Wilson <doug@somethingdoug.com> | Cookie serialization and parsing. |
| send | MIT | Copyright (c) 2012 TJ Holowaychuk<br>Copyright (c) 2014-2022 Douglas Christopher Wilson | Static file delivery. |
| serve-static | MIT | Copyright (c) 2010 Sencha Inc.<br>Copyright (c) 2011 LearnBoost<br>Copyright (c) 2011 TJ Holowaychuk<br>Copyright (c) 2014-2016 Douglas Christopher Wilson | `corvis::serve_static`. |
| etag | MIT | Copyright (c) 2014-2016 Douglas Christopher Wilson | ETag generation. |
| fresh | MIT | Copyright (c) 2012 TJ Holowaychuk <tj@vision-media.ca><br>Copyright (c) 2016-2017 Douglas Christopher Wilson <doug@somethingdoug.com> | Conditional 304 responses. |
| range-parser | MIT | Copyright (c) 2012-2014 TJ Holowaychuk <tj@vision-media.ca><br>Copyright (c) 2015-2016 Douglas Christopher Wilson <doug@somethingdoug.com | `Range` requests. |
| content-disposition | MIT | Copyright (c) 2014-2017 Douglas Christopher Wilson | `Content-Disposition` header. |
| type-is | MIT | Copyright (c) 2014 Jonathan Ong <me@jongleberry.com><br>Copyright (c) 2014-2015 Douglas Christopher Wilson <doug@somethingdoug.com> | `Content-Type` matching. |
| mime-types | MIT | Copyright (c) 2014 Jonathan Ong <me@jongleberry.com><br>Copyright (c) 2015 Douglas Christopher Wilson <doug@somethingdoug.com> | MIME type lookup. |
| mime-db | MIT | Copyright (c) 2014 Jonathan Ong <me@jongleberry.com><br>Copyright (c) 2015-2022 Douglas Christopher Wilson <doug@somethingdoug.com> | MIME type and charset data. |
| accepts | MIT | Copyright (c) 2014 Jonathan Ong <me@jongleberry.com><br>Copyright (c) 2015 Douglas Christopher Wilson <doug@somethingdoug.com> | Content negotiation. |
| negotiator | MIT | Copyright (c) 2012-2014 Federico Romero<br>Copyright (c) 2012-2014 Isaac Z. Schlueter<br>Copyright (c) 2014-2015 Douglas Christopher Wilson | Content negotiation. |
| vary | MIT | Copyright (c) 2014-2017 Douglas Christopher Wilson | `Vary` header. |
| statuses | MIT | Copyright (c) 2014 Jonathan Ong <me@jongleberry.com><br>Copyright (c) 2016 Douglas Christopher Wilson <doug@somethingdoug.com> | HTTP status code texts. |
| http-errors | MIT | Copyright (c) 2014 Jonathan Ong me@jongleberry.com<br>Copyright (c) 2016 Douglas Christopher Wilson doug@somethingdoug.com | HTTP errors of the body parser. |
| encodeurl | MIT | Copyright (c) 2016 Douglas Christopher Wilson | `Location` encoding. |
| escape-html | MIT | Copyright (c) 2012-2013 TJ Holowaychuk<br>Copyright (c) 2015 Andreas Lubbe<br>Copyright (c) 2015 Tiancheng "Timothy" Gu | HTML escaping in responses. |
| iconv-lite | MIT | Copyright (c) 2011 Alexander Shtuchkin | Charsets supported in bodies. |
| path-to-regexp | MIT | Copyright (c) 2014 Blake Embrey (hello@blakeembrey.com) | Route syntax and matching. |
| file-type | MIT | Copyright (c) Sindre Sorhus <sindresorhus@gmail.com> (https://sindresorhus.com) | Detection of the type by *magic number* (`FileTypeValidator`). |
| class-validator | MIT | Copyright (c) 2015-2020 TypeStack | Rules and messages of `corvis::validation`. |
| validator.js | MIT | Copyright (c) 2018 Chris O'Hara <cohara87@gmail.com> | Format rules of `corvis::validation`. |
| js-yaml | MIT | Copyright (C) 2011-2015 by Vitaly Puzrin | YAML serializer of `corvis-swagger`. |
| Swagger UI (`swagger-ui-dist`) | Apache-2.0 | Copyright 2020-2021 SmartBear Software Inc. | Swagger UI HTML/JS templates (`crates/corvis-swagger/src/assets/swagger-ui-*.tpl`). |
| @scalar/nestjs-api-reference | MIT | Copyright (c) 2023-present Scalar | Scalar CSS theme (`crates/corvis-swagger/src/assets/scalar-*.css`) and the reference page. |
| decimal.js | MIT | Copyright (c) 2025 Michael Mclaughlin | `corvis_orm::Decimal`: arithmetic, rounding and `toString`. |
| jsonwebtoken | MIT | Copyright (c) 2015 Auth0, Inc. <support@auth0.com> (http://auth0.com) | `corvis::jwt`: options, validation and errors. |
| jws | MIT | Copyright (c) 2013 Brian J. Brennan | JWS signing and verification. |
| cron | MIT | Copyright © 2017 Nicholas Campbell <nicholas.j.campbell@gmail.com> | Cron expressions and next-run calculation (`modules/schedule`). |
| Luxon | MIT | Copyright 2019 JS Foundation and other contributors | Time zone handling in `modules/schedule`. |
| cuid / @paralleldrive/cuid2 | MIT | Copyright (c) 2022 Eric Elliott | `cuid()` and `cuid(2)` generators of `corvis-orm`. |
| ws | MIT | Copyright (c) 2011 Einar Otto Stangvik <einaros@gmail.com><br>Copyright (c) 2013 Arnout Kazemier and contributors<br>Copyright (c) 2016 Luigi Pinca and contributors | `corvis::ws` and the WebSocket gateways. |
| supertest | MIT | Copyright (c) 2014 TJ Holowaychuk <tj@vision-media.ca> and other | Shape of the HTTP testing API (`corvis::testing`). |
| Lodash | MIT | Copyright OpenJS Foundation and other contributors <https://openjsf.org/> | Semantics of the utilities used by `corvis-swagger`. |
| V8 | BSD-3-Clause | Copyright 2014, the V8 project authors. All rights reserved. | Error messages of `JSON.parse` and JavaScript number formatting. |

## Licenses

### MIT

Applies to all the projects marked as MIT in the table, with their copyright notices:

```text
Copyright (c) 2017-present Kamil Mysliwiec <https://kamilmysliwiec.com>
Copyright (c) 2025 Colin McDonnell
Copyright (c) Ben Lorantfy <ben@lorantfy.com>
Copyright (c) 2009-2014 TJ Holowaychuk <tj@vision-media.ca>
Copyright (c) 2013-2014 Roman Shtylman <shtylman+expressjs@gmail.com>
Copyright (c) 2014-2015 Douglas Christopher Wilson <doug@somethingdoug.com>
Copyright (c) 2014 Jonathan Ong <me@jongleberry.com>
Copyright (c) 2014-2015 Douglas Christopher Wilson <doug@somethingdoug.com>
Copyright (c) 2013-2014 Jonathan Ong <me@jongleberry.com>
Copyright (c) 2014-2022 Douglas Christopher Wilson <doug@somethingdoug.com>
Copyright Brian White. All rights reserved.
Copyright (c) 2014 Hage Yaapa <[http://www.hacksparrow.com](http://www.hacksparrow.com)>
Copyright (c) 2015 Linus Unnebäck
Copyright (c) 2013 Troy Goode <troygoode@gmail.com>
Copyright (c) 2012-2014 Roman Shtylman <shtylman@gmail.com>
Copyright (c) 2015 Douglas Christopher Wilson <doug@somethingdoug.com>
Copyright (c) 2012 TJ Holowaychuk
Copyright (c) 2014-2022 Douglas Christopher Wilson
Copyright (c) 2010 Sencha Inc.
Copyright (c) 2011 LearnBoost
Copyright (c) 2011 TJ Holowaychuk
Copyright (c) 2014-2016 Douglas Christopher Wilson
Copyright (c) 2014-2016 Douglas Christopher Wilson
Copyright (c) 2012 TJ Holowaychuk <tj@vision-media.ca>
Copyright (c) 2016-2017 Douglas Christopher Wilson <doug@somethingdoug.com>
Copyright (c) 2012-2014 TJ Holowaychuk <tj@vision-media.ca>
Copyright (c) 2015-2016 Douglas Christopher Wilson <doug@somethingdoug.com
Copyright (c) 2014-2017 Douglas Christopher Wilson
Copyright (c) 2014 Jonathan Ong <me@jongleberry.com>
Copyright (c) 2014-2015 Douglas Christopher Wilson <doug@somethingdoug.com>
Copyright (c) 2014 Jonathan Ong <me@jongleberry.com>
Copyright (c) 2015 Douglas Christopher Wilson <doug@somethingdoug.com>
Copyright (c) 2014 Jonathan Ong <me@jongleberry.com>
Copyright (c) 2015-2022 Douglas Christopher Wilson <doug@somethingdoug.com>
Copyright (c) 2014 Jonathan Ong <me@jongleberry.com>
Copyright (c) 2015 Douglas Christopher Wilson <doug@somethingdoug.com>
Copyright (c) 2012-2014 Federico Romero
Copyright (c) 2012-2014 Isaac Z. Schlueter
Copyright (c) 2014-2015 Douglas Christopher Wilson
Copyright (c) 2014-2017 Douglas Christopher Wilson
Copyright (c) 2014 Jonathan Ong <me@jongleberry.com>
Copyright (c) 2016 Douglas Christopher Wilson <doug@somethingdoug.com>
Copyright (c) 2014 Jonathan Ong me@jongleberry.com
Copyright (c) 2016 Douglas Christopher Wilson doug@somethingdoug.com
Copyright (c) 2016 Douglas Christopher Wilson
Copyright (c) 2012-2013 TJ Holowaychuk
Copyright (c) 2015 Andreas Lubbe
Copyright (c) 2015 Tiancheng "Timothy" Gu
Copyright (c) 2011 Alexander Shtuchkin
Copyright (c) 2014 Blake Embrey (hello@blakeembrey.com)
Copyright (c) Sindre Sorhus <sindresorhus@gmail.com> (https://sindresorhus.com)
Copyright (c) 2015-2020 TypeStack
Copyright (c) 2018 Chris O'Hara <cohara87@gmail.com>
Copyright (C) 2011-2015 by Vitaly Puzrin
Copyright (c) 2023-present Scalar
Copyright (c) 2025 Michael Mclaughlin
Copyright (c) 2015 Auth0, Inc. <support@auth0.com> (http://auth0.com)
Copyright (c) 2013 Brian J. Brennan
Copyright © 2017 Nicholas Campbell <nicholas.j.campbell@gmail.com>
Copyright 2019 JS Foundation and other contributors
Copyright (c) 2022 Eric Elliott
Copyright (c) 2011 Einar Otto Stangvik <einaros@gmail.com>
Copyright (c) 2013 Arnout Kazemier and contributors
Copyright (c) 2016 Luigi Pinca and contributors
Copyright (c) 2014 TJ Holowaychuk <tj@vision-media.ca> and other
Copyright OpenJS Foundation and other contributors <https://openjsf.org/>
```

```text
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

### BSD-3-Clause: qs

```text
BSD 3-Clause License

Copyright (c) 2014, Nathan LaFreniere and other [contributors](https://github.com/ljharb/qs/graphs/contributors)
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### BSD-3-Clause: V8

```text
Copyright 2014, the V8 project authors. All rights reserved.
Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

    * Redistributions of source code must retain the above copyright
      notice, this list of conditions and the following disclaimer.
    * Redistributions in binary form must reproduce the above
      copyright notice, this list of conditions and the following
      disclaimer in the documentation and/or other materials provided
      with the distribution.
    * Neither the name of Google Inc. nor the names of its
      contributors may be used to endorse or promote products derived
      from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

### Apache-2.0: Prisma and Swagger UI

Prisma: Copyright Prisma Data, Inc. Swagger UI: the project `NOTICE`:

```text
swagger-ui
Copyright 2020-2021 SmartBear Software Inc.
```

```text
Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
```
